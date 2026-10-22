//! Reader for PretextMap `.pretext` files.
//!
//! A file is `pstm`, a raw-deflate header, then one raw-deflate block per
//! upper-triangular texture tile. Each tile is BC4 (RGTC1) mipmaps. The
//! overview kept for drawing is the finest mip whose full map fits in 2048
//! pixels; finer mips are decoded from the retained file when the view zooms in.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use web_time::{Duration, Instant};

use flate2::read::DeflateDecoder;
#[cfg(test)]
use flate2::write::DeflateEncoder;
#[cfg(test)]
use flate2::Compression;
#[cfg(test)]
use std::io::Write;

use crate::model::TrackKind;

const OVERVIEW_LIMIT: u32 = 2048;
const TILE_COMPRESSED_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DecodedContig {
    pub name: String,
    pub pixels: u32,
    pub ref_lo: u32,
    pub bp: u64,
    pub coverage: f32,
    pub gap: f32,
    pub repeat: f32,
    pub telomere: f32,
    pub telomere_3: f32,
    pub telomere_5: f32,
    pub pixel_density: f32,
}

struct TileSpan {
    offset: usize,
    len: usize,
    tx: u16,
    ty: u16,
}

pub struct ContactMap {
    bytes: Vec<u8>,
    tiles: Vec<TileSpan>,
    tile_at: HashMap<(u16, u16), u32>,
    texture_res: u32,
    mips: u32,
    pub(crate) overview_mip: u32,
    pub overview_side: u32,
    pub map_pixels: u32,
    pub(crate) overview: Arc<Mutex<Vec<u8>>>,
    cache: std::cell::RefCell<HashMap<(u32, u8), Vec<u8>>>,
}

impl ContactMap {
    pub fn has_source(&self) -> bool {
        !self.bytes.is_empty()
    }

    pub fn overview_at(&self, full_x: u32, full_y: u32) -> u8 {
        let side = self.overview_side.max(1);
        let map = self.map_pixels.max(1);
        let x = ((full_x as u64 * side as u64) / map as u64).min(side as u64 - 1) as usize;
        let y = ((full_y as u64 * side as u64) / map as u64).min(side as u64 - 1) as usize;
        self.overview
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(y * side as usize + x)
            .copied()
            .unwrap_or(0)
    }

    /// Sample the contact between two original-file pixels. `mip` 0 is the
    /// finest stored level.
    pub fn sample(&self, full_x: u32, full_y: u32, mip: u32) -> u8 {
        if self.map_pixels == 0 || self.texture_res == 0 || self.bytes.is_empty() {
            return self.overview_at(full_x, full_y);
        }
        let mip = mip.min(self.mips.saturating_sub(1));
        if mip >= self.overview_mip {
            return self.overview_at(full_x, full_y);
        }
        let res = self.texture_res;
        let mut px = full_x.min(self.map_pixels - 1);
        let mut py = full_y.min(self.map_pixels - 1);
        let mut tx = (px / res) as u16;
        let mut ty = (py / res) as u16;
        if tx > ty {
            std::mem::swap(&mut px, &mut py);
            std::mem::swap(&mut tx, &mut ty);
        }
        let lr = res >> mip;
        if lr == 0 {
            return 0;
        }
        let lx = (px % res) >> mip;
        let ly = (py % res) >> mip;
        let key = ((tx as u32) << 16 | ty as u32, mip as u8);
        if let Some(buf) = self.cache.borrow().get(&key) {
            return buf.get((ly * lr + lx) as usize).copied().unwrap_or(0);
        }
        let decoded = self.decode_level(tx, ty, mip, lr);
        let value = decoded.get((ly * lr + lx) as usize).copied().unwrap_or(0);
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= 48 {
            let old = cache.keys().next().copied();
            if let Some(old) = old {
                cache.remove(&old);
            }
        }
        cache.insert(key, decoded);
        value
    }

    fn decode_level(&self, tx: u16, ty: u16, mip: u32, level_res: u32) -> Vec<u8> {
        let zeros = vec![0u8; (level_res * level_res) as usize];
        let Some(&index) = self.tile_at.get(&(tx, ty)) else {
            return zeros;
        };
        let tile = &self.tiles[index as usize];
        let end = tile.offset.saturating_add(tile.len).min(self.bytes.len());
        if tile.offset >= end {
            return zeros;
        }
        let inflated = inflate_raw(&self.bytes[tile.offset..end]);
        let Ok(inflated) = inflated else {
            return zeros;
        };
        let offset = mip_byte_offset(self.texture_res, mip);
        let need = ((level_res as usize) * (level_res as usize)) / 2;
        if offset + need > inflated.len() || level_res < 4 {
            return zeros;
        }
        decode_bc4_level(&inflated, offset, level_res as usize)
    }
}

pub struct LoadedMap {
    pub genome_bp: u64,
    pub map_pixels: u32,
    pub contigs: Vec<DecodedContig>,
    pub contacts: ContactMap,
    pub tracks_on: Vec<TrackKind>,
    pub omitted: usize,
    pub failed_tiles: usize,
}

pub struct MapLoader {
    file_name: String,
    bytes: Vec<u8>,
    genome_bp: u64,
    map_pixels: u32,
    texture_res: u32,
    textures_1d: u32,
    mips: u32,
    overview_mip: u32,
    src_res: u32,
    tiles: Vec<TileSpan>,
    next_tile: usize,
    failed_tiles: usize,
    overview: Arc<Mutex<Vec<u8>>>,
    extension_at: usize,
    contigs_raw: Vec<RawContig>,
}

struct RawContig {
    name: String,
    pixel_end: u32,
    bp: u64,
}

pub enum Pump {
    Loading,
    Ready,
}

impl MapLoader {
    pub fn open(file_name: String, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() < 12 || &bytes[..4] != b"pstm" {
            return Err("that file does not start with the pretext magic".to_string());
        }
        let header_comp = u32_at(&bytes, 4)? as usize;
        let header_len = u32_at(&bytes, 8)? as usize;
        if header_comp == 0 || header_comp > 64 * 1024 * 1024 || 12 + header_comp > bytes.len() {
            return Err("the pretext header size is not usable".to_string());
        }
        let header = inflate_raw(&bytes[12..12 + header_comp])?;
        if header.len() != header_len {
            return Err(format!(
                "header decompressed to {} bytes, the file said {header_len}",
                header.len()
            ));
        }
        if header.len() < 15 {
            return Err("the pretext header is too small".to_string());
        }
        let genome_bp = u64_at(&header, 0)?;
        let n_contigs = u32_at(&header, 8)? as usize;
        let records = 12 + n_contigs * 68;
        if header.len() < records + 3 {
            return Err("the pretext header does not cover its contig records".to_string());
        }
        let map_trailer = records;
        let texture_res_log = header[map_trailer] as u32;
        let n_text_log = header[map_trailer + 1] as u32;
        let mips = header[map_trailer + 2] as u32;
        if !(1..=16).contains(&texture_res_log) || n_text_log > 8 || !(1..=12).contains(&mips) {
            return Err("the pretext texture dimensions are not ones this viewer can draw".to_string());
        }
        if mips > texture_res_log {
            return Err("the pretext file has more mip levels than its texture size".to_string());
        }
        let texture_res = 1u32 << texture_res_log;
        let textures_1d = 1u32 << n_text_log;
        let map_pixels = texture_res.saturating_mul(textures_1d);
        if map_pixels == 0 {
            return Err("the pretext map has no pixels".to_string());
        }
        let mut overview_mip = mips - 1;
        while overview_mip > 0 {
            let finer = texture_res >> (overview_mip - 1);
            if finer < 4 || textures_1d.saturating_mul(finer) > OVERVIEW_LIMIT {
                break;
            }
            overview_mip -= 1;
        }
        let src_res = texture_res >> overview_mip;
        if src_res < 4 {
            return Err("the pretext mip chain is finer than a BC4 block".to_string());
        }
        let overview_side = textures_1d * src_res;
        if overview_side == 0 || overview_side > OVERVIEW_LIMIT {
            return Err("the pretext overview does not fit in memory".to_string());
        }

        let mut contigs_raw = Vec::with_capacity(n_contigs);
        let mut cursor = 12usize;
        let mut frac_acc = 0f64;
        for i in 0..n_contigs {
            let frac = f32_at(&header, cursor)? as f64;
            cursor += 4;
            let name = c_string(&header[cursor..cursor + 64], &format!("sequence_{i}"));
            cursor += 64;
            frac_acc += frac.max(0.0);
            let pixel_end = (frac_acc * map_pixels as f64).floor().min(map_pixels as f64) as u32;
            let bp = (frac.max(0.0) * genome_bp as f64).round() as u64;
            contigs_raw.push(RawContig {
                name,
                pixel_end,
                bp,
            });
        }
        if let Some(last) = contigs_raw.last_mut() {
            last.pixel_end = map_pixels;
        }

        let mut tiles = Vec::new();
        let mut offset = 12 + header_comp;
        for tx in 0..textures_1d {
            for ty in tx..textures_1d {
                if offset + 4 > bytes.len() {
                    break;
                }
                let len = u32_at(&bytes, offset)? as usize;
                offset += 4;
                if len > TILE_COMPRESSED_LIMIT || offset + len > bytes.len() {
                    break;
                }
                tiles.push(TileSpan {
                    offset,
                    len,
                    tx: tx as u16,
                    ty: ty as u16,
                });
                offset += len;
            }
        }
        if tiles.is_empty() {
            return Err("the pretext file has no texture tiles".to_string());
        }
        let extension_at = offset;
        Ok(Self {
            file_name,
            bytes,
            genome_bp,
            map_pixels,
            texture_res,
            textures_1d,
            mips,
            overview_mip,
            src_res,
            tiles,
            next_tile: 0,
            failed_tiles: 0,
            overview: Arc::new(Mutex::new(vec![0u8; (overview_side as usize) * (overview_side as usize)])),
            extension_at,
            contigs_raw,
        })
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Header, sequence list, and the overview buffer. Tiles are still empty
    /// and fill that buffer as `pump` runs.
    pub fn preview(&self) -> LoadedMap {
        let (contigs, omitted) = self.layout_contigs(false);
        LoadedMap {
            genome_bp: self.genome_bp,
            map_pixels: self.map_pixels,
            contigs,
            contacts: self.contacts_shell(),
            tracks_on: Vec::new(),
            omitted,
            failed_tiles: 0,
        }
    }

    fn contacts_shell(&self) -> ContactMap {
        ContactMap {
            bytes: Vec::new(),
            tiles: Vec::new(),
            tile_at: HashMap::new(),
            texture_res: self.texture_res,
            mips: self.mips,
            overview_mip: self.overview_mip,
            overview_side: self.textures_1d * self.src_res,
            map_pixels: self.map_pixels,
            overview: Arc::clone(&self.overview),
            cache: std::cell::RefCell::new(HashMap::new()),
        }
    }

    fn layout_contigs(&self, with_density: bool) -> (Vec<DecodedContig>, usize) {
        let mut assigned = 0u32;
        let mut contigs = Vec::new();
        let mut omitted = 0usize;
        let side = self.textures_1d * self.src_res;
        for (i, raw) in self.contigs_raw.iter().enumerate() {
            let end = if i + 1 == self.contigs_raw.len() {
                self.map_pixels
            } else {
                raw.pixel_end.min(self.map_pixels).max(assigned)
            };
            let pixels = end.saturating_sub(assigned);
            if pixels == 0 {
                omitted += 1;
                continue;
            }
            let density = if with_density {
                let overview = self.overview.lock().unwrap_or_else(|err| err.into_inner());
                diagonal_mean(&overview, side, self.map_pixels, assigned, end)
            } else {
                0.0
            };
            contigs.push(DecodedContig {
                name: raw.name.clone(),
                pixels,
                ref_lo: assigned,
                bp: raw.bp.max(1),
                coverage: 0.0,
                gap: 0.0,
                repeat: 0.0,
                telomere: 0.0,
                telomere_3: 0.0,
                telomere_5: 0.0,
                pixel_density: density,
            });
            assigned = end;
        }
        (contigs, omitted)
    }

    pub fn progress(&self) -> String {
        format!(
            "Decoding {}  {}/{} tiles",
            self.file_name,
            self.next_tile,
            self.tiles.len()
        )
    }

    pub fn pump(&mut self) -> Result<Pump, String> {
        let start = Instant::now();
        let budget = Duration::from_millis(24);
        while self.next_tile < self.tiles.len() && start.elapsed() < budget {
            self.decode_next();
        }
        if self.next_tile < self.tiles.len() {
            Ok(Pump::Loading)
        } else {
            Ok(Pump::Ready)
        }
    }

    fn decode_next(&mut self) {
        let index = self.next_tile;
        self.next_tile += 1;
        let tile = &self.tiles[index];
        let end = tile.offset.saturating_add(tile.len).min(self.bytes.len());
        let inflated = inflate_raw(&self.bytes[tile.offset..end]);
        let Ok(inflated) = inflated else {
            self.failed_tiles += 1;
            return;
        };
        let offset = mip_byte_offset(self.texture_res, self.overview_mip);
        let need = (self.src_res as usize) * (self.src_res as usize) / 2;
        if offset + need > inflated.len() {
            self.failed_tiles += 1;
            return;
        }
        let level = decode_bc4_level(&inflated, offset, self.src_res as usize);
        let mut overview = self.overview.lock().unwrap_or_else(|err| err.into_inner());
        blit_tile(
            &mut overview,
            self.textures_1d * self.src_res,
            tile.tx as u32,
            tile.ty as u32,
            self.src_res,
            &level,
        );
    }

    pub fn finish(self) -> Result<LoadedMap, String> {
        let (mut contigs, omitted) = self.layout_contigs(true);
        if contigs.is_empty() {
            return Err("the pretext header listed no sequences that cover the map".to_string());
        }
        let mut tracks_on = Vec::new();
        if let Err(err) = apply_extensions(&self.bytes, self.extension_at, self.map_pixels, &mut contigs, &mut tracks_on) {
            // The contact map is already decoded. A bad graph block should not hide it.
            let _ = err;
        }
        let mut tile_at = HashMap::with_capacity(self.tiles.len());
        for (i, tile) in self.tiles.iter().enumerate() {
            tile_at.insert((tile.tx, tile.ty), i as u32);
        }
        let contacts = ContactMap {
            bytes: self.bytes,
            tiles: self.tiles,
            tile_at,
            texture_res: self.texture_res,
            mips: self.mips,
            overview_mip: self.overview_mip,
            overview_side: self.textures_1d * self.src_res,
            map_pixels: self.map_pixels,
            overview: Arc::clone(&self.overview),
            cache: std::cell::RefCell::new(HashMap::new()),
        };
        Ok(LoadedMap {
            genome_bp: self.genome_bp,
            map_pixels: self.map_pixels,
            contigs,
            contacts,
            tracks_on,
            omitted,
            failed_tiles: self.failed_tiles,
        })
    }
}

fn apply_extensions(
    bytes: &[u8],
    mut offset: usize,
    map_pixels: u32,
    contigs: &mut [DecodedContig],
    tracks_on: &mut Vec<TrackKind>,
) -> Result<(), String> {
    let mut guard = 0;
    while offset + 8 <= bytes.len() && guard < 64 {
        guard += 1;
        if &bytes[offset..offset + 4] != b"psgh" {
            if let Some(found) = bytes[offset..].windows(4).position(|w| w == b"psgh") {
                offset += found;
                continue;
            }
            break;
        }
        offset += 4;
        let len = u32_at(bytes, offset)? as usize;
        offset += 4;
        if len > TILE_COMPRESSED_LIMIT || offset + len > bytes.len() {
            break;
        }
        let inflated = inflate_raw(&bytes[offset..offset + len])?;
        offset += len;
        let values_bytes = 4usize * map_pixels as usize;
        if inflated.len() < 64 + values_bytes {
            continue;
        }
        let name = c_string(&inflated[..64], "graph");
        let Some(kind) = kind_for(&name) else {
            continue;
        };
        let mut values = vec![0f32; map_pixels as usize];
        let mut max = 1i32;
        for (i, chunk) in inflated[64..64 + values_bytes].chunks_exact(4).enumerate() {
            let v = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if v > max {
                max = v;
            }
            values[i] = v.max(0) as f32;
        }
        let max = max as f32;
        for contig in contigs.iter_mut() {
            let start = contig.ref_lo as usize;
            let end = (contig.ref_lo + contig.pixels) as usize;
            let slice = &values[start.min(values.len())..end.min(values.len())];
            let mean = if slice.is_empty() {
                0.0
            } else {
                slice.iter().sum::<f32>() / slice.len() as f32 / max
            };
            match kind {
                TrackKind::Coverage => contig.coverage = mean,
                TrackKind::Gap => contig.gap = mean,
                TrackKind::Repeat => contig.repeat = mean,
                TrackKind::Telomere => contig.telomere = mean,
                TrackKind::Telomere3 => contig.telomere_3 = mean,
                TrackKind::Telomere5 => contig.telomere_5 = mean,
                TrackKind::PixelDensity => contig.pixel_density = mean,
            }
        }
        if !tracks_on.contains(&kind) {
            tracks_on.push(kind);
        }
    }
    Ok(())
}

fn kind_for(name: &str) -> Option<TrackKind> {
    let n = name.to_ascii_lowercase();
    if n.contains("telomere") && (n.contains("3p") || n.contains('3')) && !n.contains("5p") {
        Some(TrackKind::Telomere3)
    } else if n.contains("telomere") && (n.contains("5p") || n.contains('5')) {
        Some(TrackKind::Telomere5)
    } else if n.contains("telomere") {
        Some(TrackKind::Telomere)
    } else if n.contains("gap") {
        Some(TrackKind::Gap)
    } else if n.contains("repeat") {
        Some(TrackKind::Repeat)
    } else if n.contains("coverage") || n.contains("cov") {
        Some(TrackKind::Coverage)
    } else if n.contains("density") {
        Some(TrackKind::PixelDensity)
    } else {
        None
    }
}

fn diagonal_mean(overview: &[u8], side: u32, map_pixels: u32, start: u32, end: u32) -> f32 {
    if side == 0 || map_pixels == 0 || end <= start {
        return 0.0;
    }
    let a = ((start as u64 * side as u64) / map_pixels as u64).min(side as u64 - 1) as u32;
    let b = ((end as u64 * side as u64) / map_pixels as u64).min(side as u64) as u32;
    if b <= a {
        let i = (a * side + a) as usize;
        return overview.get(i).copied().unwrap_or(0) as f32 / 255.0;
    }
    let mut sum = 0u64;
    for p in a..b {
        sum += overview[(p * side + p) as usize] as u64;
    }
    sum as f32 / (b - a) as f32 / 255.0
}

fn blit_tile(overview: &mut [u8], side: u32, tx: u32, ty: u32, src_res: u32, level: &[u8]) {
    for py in 0..src_res {
        for px in 0..src_res {
            let val = level[(py * src_res + px) as usize];
            let gx = tx * src_res + px;
            let gy = ty * src_res + py;
            if gx < side && gy < side {
                overview[(gy * side + gx) as usize] = val;
                if tx != ty {
                    overview[(gx * side + gy) as usize] = val;
                }
            }
        }
    }
}

fn mip_byte_offset(texture_res: u32, mip: u32) -> usize {
    let mut offset = 0usize;
    let mut res = texture_res;
    for _ in 0..mip {
        offset += (res as usize) * (res as usize) / 2;
        res >>= 1;
    }
    offset
}

fn decode_bc4_level(data: &[u8], mut offset: usize, resolution: usize) -> Vec<u8> {
    let mut output = vec![0u8; resolution * resolution];
    let blocks = resolution / 4;
    let mut palette = [0u8; 8];
    let mut pixels = [0u8; 16];
    for bx in 0..blocks {
        for by in 0..blocks {
            if offset + 8 > data.len() {
                return output;
            }
            decode_bc4_block(&data[offset..offset + 8], &mut palette, &mut pixels);
            offset += 8;
            for dx in 0..4 {
                for dy in 0..4 {
                    let px = bx * 4 + dx;
                    let py = by * 4 + dy;
                    output[py * resolution + px] = pixels[dx * 4 + dy];
                }
            }
        }
    }
    output
}

fn decode_bc4_block(data: &[u8], palette: &mut [u8; 8], pixels: &mut [u8; 16]) {
    let a0 = data[0] as u32;
    let a1 = data[1] as u32;
    palette[0] = data[0];
    palette[1] = data[1];
    if a0 > a1 {
        palette[2] = lerp(a0, a1, 6, 7);
        palette[3] = lerp(a0, a1, 5, 7);
        palette[4] = lerp(a0, a1, 4, 7);
        palette[5] = lerp(a0, a1, 3, 7);
        palette[6] = lerp(a0, a1, 2, 7);
        palette[7] = lerp(a0, a1, 1, 7);
    } else {
        palette[2] = lerp(a0, a1, 4, 5);
        palette[3] = lerp(a0, a1, 3, 5);
        palette[4] = lerp(a0, a1, 2, 5);
        palette[5] = lerp(a0, a1, 1, 5);
        palette[6] = 0;
        palette[7] = 255;
    }
    let lo = data[2] as u32 | ((data[3] as u32) << 8) | ((data[4] as u32) << 16);
    let hi = data[5] as u32 | ((data[6] as u32) << 8) | ((data[7] as u32) << 16);
    for i in 0..8 {
        pixels[i] = palette[((lo >> (i * 3)) & 7) as usize];
        pixels[8 + i] = palette[((hi >> (i * 3)) & 7) as usize];
    }
}

fn lerp(a0: u32, a1: u32, n: u32, d: u32) -> u8 {
    ((n * a0 + (d - n) * a1 + d / 2) / d) as u8
}

fn inflate_raw(src: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = DeflateDecoder::new(src);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|err| format!("could not inflate pretext data ({err})"))?;
    Ok(out)
}

#[cfg(test)]
fn deflate_raw(src: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(src).expect("deflate");
    encoder.finish().expect("deflate finish")
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let chunk = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "pretext file ended early".to_string())?;
    Ok(u32::from_le_bytes(chunk.try_into().unwrap()))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let chunk = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "pretext file ended early".to_string())?;
    Ok(u64::from_le_bytes(chunk.try_into().unwrap()))
}

fn f32_at(bytes: &[u8], offset: usize) -> Result<f32, String> {
    Ok(f32::from_le_bytes(u32_at(bytes, offset)?.to_le_bytes()))
}

fn c_string(bytes: &[u8], fallback: &str) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(contigs: &[(&str, f32)], genome: u64, tex_log: u8, n_log: u8, mips: u8) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend_from_slice(&genome.to_le_bytes());
        raw.extend_from_slice(&(contigs.len() as u32).to_le_bytes());
        for (name, frac) in contigs {
            raw.extend_from_slice(&frac.to_le_bytes());
            let mut buf = [0u8; 64];
            let bytes = name.as_bytes();
            buf[..bytes.len().min(63)].copy_from_slice(&bytes[..bytes.len().min(63)]);
            raw.extend_from_slice(&buf);
        }
        raw.push(tex_log);
        raw.push(n_log);
        raw.push(mips);
        raw
    }

    fn bc4_solid(value: u8) -> [u8; 8] {
        // alpha0 > alpha1 and every index 0, so the 4×4 block is `value`.
        if value > 0 {
            [value, 0, 0, 0, 0, 0, 0, 0]
        } else {
            [0, 1, 0, 0, 0, 0, 0, 0]
        }
    }

    fn pack(header_raw: &[u8], tiles: &[Vec<u8>]) -> Vec<u8> {
        let header_comp = deflate_raw(header_raw);
        let mut file = b"pstm".to_vec();
        file.extend_from_slice(&(header_comp.len() as u32).to_le_bytes());
        file.extend_from_slice(&(header_raw.len() as u32).to_le_bytes());
        file.extend_from_slice(&header_comp);
        for tile in tiles {
            let comp = deflate_raw(tile);
            file.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            file.extend_from_slice(&comp);
        }
        file
    }

    fn load(bytes: Vec<u8>) -> LoadedMap {
        let mut loader = MapLoader::open("tiny.pretext".to_string(), bytes).unwrap();
        loop {
            match loader.pump().unwrap() {
                Pump::Loading => {}
                Pump::Ready => break,
            }
        }
        loader.finish().unwrap()
    }

    fn pixels(map: &ContactMap) -> Vec<u8> {
        map.overview.lock().unwrap().clone()
    }

    #[test]
    fn a_real_pretext_file_decodes() {
        let path = std::env::var("PRETEXT_FILE").unwrap_or_else(|_| "/tmp/Anilios_waitii.pretext".to_string());
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let bytes = std::fs::read(path).unwrap();
        let mut loader = MapLoader::open("Anilios_waitii.pretext".into(), bytes).unwrap();
        eprintln!(
            "tiles {} map {} side preview contigs {}",
            loader.tile_count(),
            loader.map_pixels,
            loader.preview().contigs.len()
        );
        let mut steps = 0;
        loop {
            steps += 1;
            match loader.pump().unwrap() {
                Pump::Loading => {}
                Pump::Ready => break,
            }
            if steps == 3 {
                let preview = loader.preview();
                let image = pixels(&preview.contacts);
                let nonzero = image.iter().filter(|p| **p > 0).count();
                eprintln!("after 3 pumps nonzero {nonzero} / {}", image.len());
            }
        }
        let loaded = loader.finish().unwrap();
        let image = pixels(&loaded.contacts);
        let nonzero = image.iter().filter(|p| **p > 0).count();
        let max = image.iter().copied().max().unwrap_or(0);
        let side = (image.len() as f64).sqrt() as usize;
        let mut diagonal = 0u64;
        let mut top = 0u64;
        for i in 0..side {
            diagonal += image[i * side + i] as u64;
            top += image[i] as u64;
        }
        eprintln!(
            "diagonal avg {:.1} top avg {:.1}",
            diagonal as f64 / side as f64,
            top as f64 / side as f64
        );
        assert!(
            diagonal > top,
            "decoded image has no contact diagonal"
        );
        eprintln!(
            "done contigs {} omitted {} failed {} nonzero {} max {max} genome {}",
            loaded.contigs.len(),
            loaded.omitted,
            loaded.failed_tiles,
            nonzero,
            loaded.genome_bp
        );
        assert!(nonzero > 1000, "real map decoded to an empty image");
        assert_eq!(loaded.failed_tiles, 0);
    }

    #[test]
    fn a_solid_tile_fills_the_overview() {
        // textureRes 4, one tile, one mip. Overview is the 4×4 block itself.
        let file = pack(&header(&[("chr1", 1.0)], 1_000, 2, 0, 1), &[bc4_solid(200).to_vec()]);
        let loaded = load(file);
        assert_eq!(loaded.map_pixels, 4);
        assert_eq!(loaded.contacts.overview_side, 4);
        assert_eq!(loaded.contigs.len(), 1);
        assert_eq!(loaded.contigs[0].name, "chr1");
        assert_eq!(loaded.contigs[0].pixels, 4);
        assert_eq!(loaded.genome_bp, 1_000);
        assert!(pixels(&loaded.contacts).iter().all(|p| *p == 200));
        assert_eq!(loaded.contacts.sample(1, 2, 0), 200);
    }

    #[test]
    fn an_off_diagonal_tile_is_mirrored() {
        // Two tiles on a side. Only tile (0, 1) is bright.
        let dark = bc4_solid(0).to_vec();
        let bright = bc4_solid(255).to_vec();
        let file = pack(
            &header(&[("a", 0.5), ("b", 0.5)], 8_000, 2, 1, 1),
            &[dark.clone(), bright, dark],
        );
        let loaded = load(file);
        assert_eq!(loaded.map_pixels, 8);
        assert_eq!(loaded.contacts.overview_side, 8);
        assert_eq!(loaded.contigs.len(), 2);
        let side = 8usize;
        let image = pixels(&loaded.contacts);
        let at = |x, y| image[y * side + x];
        assert_eq!(at(1, 1), 0);
        assert_eq!(at(1, 5), 255);
        assert_eq!(at(5, 1), 255);
        assert_eq!(at(6, 6), 0);
    }

    #[test]
    fn column_major_bc4_pixels_keep_their_place() {
        // Index 1 (bits 3..5) selects alpha0; every other index selects alpha1.
        // That pixel is (dx=0, dy=1) inside the block.
        let mut block = [180u8, 10, 0, 0, 0, 0, 0, 0];
        let mut bits = 0u64;
        for i in 0..16 {
            let index = if i == 1 { 0u64 } else { 1 };
            bits |= index << (i * 3);
        }
        block[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
        let file = pack(&header(&[("chr1", 1.0)], 100, 2, 0, 1), &[block.to_vec()]);
        let loaded = load(file);
        let side = 4usize;
        let image = pixels(&loaded.contacts);
        let at = |x, y| image[y * side + x];
        assert_eq!(at(0, 1), 180);
        assert_eq!(at(0, 0), 10);
        assert_eq!(at(1, 0), 10);
    }
}
