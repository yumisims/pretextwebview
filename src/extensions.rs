//! Graph extensions stored after the contact textures in a `.pretext` file.
//!
//! PretextGraph writes each track as a `psgh` block: a 64-byte name and one
//! `i32` sample per map pixel. Coverage, gap, repeat density, telomeres,
//! bedgraph, bigWig, and `.alg` tracks all arrive this way. The name is the
//! one PretextGraph was given, and every block is kept.

use std::io::Read;

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use flate2::read::DeflateDecoder;

const COMPRESSED_LIMIT: usize = 32 * 1024 * 1024;
const GRAPH_LIMIT: usize = 256;

// ---------------------------------------------------------------------------
// Names shared by the file, the keyboard, and the built-in demo tracks.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Coverage,
    Gap,
    Repeat,
    Telomere,
    Telomere3,
    Telomere5,
    PixelDensity,
}

/// Built-in name, matched loosely so `Coverage` and `cov` still fill the
/// per-contig fields. Custom bedgraph, bigWig, and alg names return `None`
/// and are kept as their own series.
pub fn kind_for(name: &str) -> Option<TrackKind> {
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

pub fn shortcut(name: &str) -> Option<&'static str> {
    match name {
        "coverage" => Some("C"),
        "gap" => Some("G"),
        "repeat_density" => Some("R"),
        "telomere" => Some("T"),
        "3p_telomere" => Some("3"),
        "5p_telomere" => Some("5"),
        _ => None,
    }
}

/// PretextView colours for the named tracks. Anything else starts from the
/// PretextView teal and then rotates, so stacked bedgraph / bigWig / alg
/// tracks stay separable.
pub fn color_for(name: &str, index: usize) -> [u8; 4] {
    const EXTRA: [[u8; 4]; 6] = [
        [26, 204, 179, 255],
        [80, 160, 210, 255],
        [220, 140, 60, 255],
        [180, 90, 160, 255],
        [120, 190, 90, 255],
        [210, 90, 90, 255],
    ];
    match name.to_ascii_lowercase().as_str() {
        "3p_telomere" => [221, 204, 119, 255],
        "5p_telomere" => [136, 204, 238, 255],
        "telomere" => [200, 160, 80, 255],
        "coverage" => [80, 180, 120, 255],
        "gap" => [220, 80, 80, 255],
        "repeat_density" => [160, 100, 200, 255],
        "pixel_density" => [80, 160, 210, 255],
        _ => EXTRA[index % EXTRA.len()],
    }
}

fn checkbox_label(name: &str) -> String {
    match shortcut(name) {
        Some(key) => format!("{name}  ({key})"),
        None => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// `psgh` blocks in the file.
// ---------------------------------------------------------------------------

/// One graph as stored in the file, samples scaled to 0..=1.
#[derive(Clone, Debug)]
pub struct LoadedGraph {
    pub name: String,
    pub values: Vec<f32>,
}

pub struct Graphs {
    pub graphs: Vec<LoadedGraph>,
    pub tracks_on: Vec<TrackKind>,
}

/// A sequence span that can receive the mean of a recognised track.
pub trait ContigSpan {
    fn pixel_range(&self) -> (u32, u32);
    fn set_mean(&mut self, kind: TrackKind, mean: f32);
}

/// Read every `psgh` block from `offset` to the end of the texture directory.
/// Unrecognised names are kept. A bad block stops the scan; earlier graphs
/// are still returned.
pub fn read_graphs<C: ContigSpan>(
    bytes: &[u8],
    mut offset: usize,
    map_pixels: u32,
    contigs: &mut [C],
) -> Graphs {
    let pixels = map_pixels.max(1) as usize;
    let mut graphs = Vec::new();
    let mut tracks_on = Vec::new();
    let mut guard = 0;
    while offset + 8 <= bytes.len() && guard < GRAPH_LIMIT {
        guard += 1;
        if &bytes[offset..offset + 4] != b"psgh" {
            if let Some(found) = bytes[offset..].windows(4).position(|window| window == b"psgh") {
                offset += found;
                continue;
            }
            break;
        }
        offset += 4;
        let Ok(len) = u32_at(bytes, offset) else {
            break;
        };
        let len = len as usize;
        offset += 4;
        if len > COMPRESSED_LIMIT || offset + len > bytes.len() {
            break;
        }
        let compressed = &bytes[offset..offset + len];
        offset += len;
        let Ok(inflated) = inflate_raw(compressed) else {
            continue;
        };
        if inflated.len() < 68 {
            continue;
        }
        let name = c_string(&inflated[..64.min(inflated.len())]);
        let count = inflated.len().saturating_sub(64) / 4;
        let count = count.min(pixels);
        let mut raw = vec![0f32; pixels];
        let mut max = 1i32;
        for (i, chunk) in inflated[64..64 + count * 4].chunks_exact(4).enumerate() {
            let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if value > max {
                max = value;
            }
            raw[i] = value.max(0) as f32;
        }
        let max = max as f32;
        let values: Vec<f32> = raw.iter().map(|value| *value / max).collect();
        if let Some(kind) = kind_for(&name) {
            for contig in contigs.iter_mut() {
                let (start, end) = contig.pixel_range();
                contig.set_mean(kind, mean_of(&values, start, end));
            }
            if !tracks_on.contains(&kind) {
                tracks_on.push(kind);
            }
        }
        graphs.push(LoadedGraph { name, values });
    }
    Graphs { graphs, tracks_on }
}

fn mean_of(values: &[f32], start: u32, end: u32) -> f32 {
    let start = start as usize;
    let end = end as usize;
    let slice = &values[start.min(values.len())..end.min(values.len())];
    if slice.is_empty() {
        0.0
    } else {
        slice.iter().sum::<f32>() / slice.len() as f32
    }
}

// ---------------------------------------------------------------------------
// Tracks the session can show.
// ---------------------------------------------------------------------------

/// Per-pixel series from the file. Drawn in the current display order, so
/// moves and inverts follow the sequence.
#[derive(Clone, Debug)]
pub struct GraphSeries {
    pub name: String,
    pub on: bool,
    pub color: [u8; 4],
    pub values: Vec<f32>,
}

/// Flat built-in track. Used for the demo map, which has no `psgh` blocks,
/// and kept in step with a file graph of the same name.
#[derive(Clone, Debug)]
pub struct Track {
    pub name: String,
    pub on: bool,
    pub color: [u8; 4],
    pub height: f32,
    pub scale: f32,
    pub line: f32,
    pub label: bool,
    pub kind: TrackKind,
}

pub fn builtin_tracks() -> Vec<Track> {
    vec![
        track("3p_telomere", TrackKind::Telomere3, false),
        track("5p_telomere", TrackKind::Telomere5, false),
        track("telomere", TrackKind::Telomere, false),
        track("coverage", TrackKind::Coverage, false),
        track("gap", TrackKind::Gap, false),
        track("repeat_density", TrackKind::Repeat, false),
        track("pixel_density", TrackKind::PixelDensity, true),
    ]
}

fn track(name: &str, kind: TrackKind, on: bool) -> Track {
    Track {
        name: name.to_string(),
        on,
        color: color_for(name, 0),
        height: 1.0,
        scale: 1.0,
        line: 1.5,
        label: true,
        kind,
    }
}

/// File graphs start off, matching PretextView (`gph->on = 0`).
pub fn series_from(graphs: Vec<LoadedGraph>) -> Vec<GraphSeries> {
    graphs
        .into_iter()
        .enumerate()
        .map(|(index, graph)| GraphSeries {
            color: color_for(&graph.name, index),
            name: graph.name,
            on: false,
            values: graph.values,
        })
        .collect()
}

pub fn sync_builtin(tracks: &mut [Track], graphs: &[GraphSeries]) {
    for track in tracks {
        track.on = graphs.iter().any(|graph| graph.name == track.name && graph.on);
    }
}

/// Flip a file graph of this name, or the built-in track when the map has
/// no such graph. Returns the new state.
pub fn toggle(graphs: &mut [GraphSeries], tracks: &mut [Track], name: &str) -> Option<bool> {
    if let Some(graph) = graphs.iter_mut().find(|graph| graph.name == name) {
        graph.on = !graph.on;
        let on = graph.on;
        if let Some(track) = tracks.iter_mut().find(|track| track.name == name) {
            track.on = on;
        }
        return Some(on);
    }
    let track = tracks.iter_mut().find(|track| track.name == name)?;
    track.on = !track.on;
    Some(track.on)
}

pub fn load_note(graphs: &[GraphSeries]) -> String {
    if graphs.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = graphs.iter().take(8).map(|graph| graph.name.as_str()).collect();
    let mut note = format!(" — {} extension tracks ({})", graphs.len(), shown.join(", "));
    if graphs.len() > shown.len() {
        note.push_str(", …");
    }
    note
}

// ---------------------------------------------------------------------------
// Drawing along the top of the map, in display order.
// ---------------------------------------------------------------------------

pub trait MapPiece {
    fn pixels(&self) -> u32;
    fn ref_at(&self, local: u32) -> i32;
    fn mean(&self, kind: TrackKind) -> f32;
}

pub fn paint<P: MapPiece>(
    painter: &egui::Painter,
    rect: Rect,
    camera_x: f32,
    scale: f32,
    map_pixels: u32,
    graphs: &[GraphSeries],
    tracks: &[Track],
    pieces: &[P],
) {
    if !graphs.is_empty() {
        paint_graphs(painter, rect, camera_x, scale, map_pixels, graphs, pieces);
    } else {
        paint_builtin(painter, rect, camera_x, scale, map_pixels, tracks, pieces);
    }
}

fn paint_graphs<P: MapPiece>(
    painter: &egui::Painter,
    rect: Rect,
    camera_x: f32,
    scale: f32,
    map_pixels: u32,
    graphs: &[GraphSeries],
    pieces: &[P],
) {
    let mut shown = 0usize;
    for graph in graphs.iter().filter(|graph| graph.on) {
        let top = rect.top() + 6.0 + shown as f32 * 24.0;
        if top > rect.bottom() {
            break;
        }
        let color = rgba(graph.color);
        let mut run = Vec::new();
        let mut x = rect.left();
        while x <= rect.right() {
            match sample(graph, pieces, map_pixels, camera_x, rect, scale, x) {
                Some(value) => {
                    let y = top + 18.0 * (1.0 - value.clamp(0.0, 1.0));
                    run.push(Pos2::new(x, y));
                }
                None => {
                    stroke(painter, &mut run, color);
                }
            }
            x += 2.0;
        }
        stroke(painter, &mut run, color);
        painter.text(
            Pos2::new(rect.left() + 8.0, top),
            egui::Align2::LEFT_TOP,
            &graph.name,
            egui::FontId::proportional(12.0),
            color,
        );
        shown += 1;
    }
}

fn sample<P: MapPiece>(
    graph: &GraphSeries,
    pieces: &[P],
    map_pixels: u32,
    camera_x: f32,
    rect: Rect,
    scale: f32,
    screen_x: f32,
) -> Option<f32> {
    let display = display_pixel(camera_x, rect, scale, map_pixels, screen_x)?;
    let original = original_at(pieces, map_pixels, display)?;
    graph.values.get(original).copied()
}

fn stroke(painter: &egui::Painter, run: &mut Vec<Pos2>, color: Color32) {
    if run.len() >= 2 {
        painter.line(std::mem::take(run), Stroke::new(1.5_f32, color));
    } else {
        run.clear();
    }
}

fn paint_builtin<P: MapPiece>(
    painter: &egui::Painter,
    rect: Rect,
    camera_x: f32,
    scale: f32,
    map_pixels: u32,
    tracks: &[Track],
    pieces: &[P],
) {
    let tracks: Vec<&Track> = tracks.iter().filter(|track| track.on).collect();
    if tracks.is_empty() || map_pixels == 0 {
        return;
    }
    let mut acc = 0u32;
    for piece in pieces {
        let start = acc;
        acc += piece.pixels();
        let x0 = screen_x(rect, camera_x, scale, map_pixels, start);
        let x1 = screen_x(rect, camera_x, scale, map_pixels, acc);
        for (index, track) in tracks.iter().enumerate() {
            let value = piece.mean(track.kind).clamp(0.0, 1.0);
            let height = 8.0 * track.height.max(0.4);
            let top = rect.top() + index as f32 * (height + 2.0);
            let filled = Rect::from_min_max(
                Pos2::new(x0, top),
                Pos2::new(x1, top + height * value.max(0.08)),
            );
            painter.rect_filled(filled, 0.0, rgba(track.color));
        }
    }
}

pub fn paint_overlay(
    painter: &egui::Painter,
    rect: Rect,
    graphs: &[GraphSeries],
    tracks: &[Track],
    visible: bool,
) {
    if !visible {
        return;
    }
    let mut lines = vec!["Extensions".to_string(), "X: exit".to_string()];
    let source: Vec<&str> = if graphs.is_empty() {
        tracks.iter().map(|track| track.name.as_str()).collect()
    } else {
        graphs.iter().map(|graph| graph.name.as_str()).collect()
    };
    for name in source {
        match shortcut(name) {
            Some(key) => lines.push(format!("{key}: {name}")),
            None => lines.push(name.to_string()),
        }
    }
    let text = lines.join("\n");
    let color = Color32::WHITE;
    let galley = painter.layout(
        text,
        egui::FontId::proportional(14.0),
        color,
        280.0,
    );
    let pad = 8.0;
    let size = galley.size() + Vec2::splat(pad * 2.0);
    let origin = rect.right_top() + Vec2::new(-size.x - 12.0, 12.0);
    let frame = Rect::from_min_size(origin, size);
    painter.rect_filled(frame, 6.0, Color32::from_rgb(12, 14, 18));
    painter.rect_stroke(
        frame,
        6.0,
        Stroke::new(1.0_f32, Color32::from_rgb(230, 230, 230)),
        egui::StrokeKind::Inside,
    );
    painter.galley(frame.min + Vec2::splat(pad), galley, color);
}

fn screen_x(rect: Rect, camera_x: f32, scale: f32, map_pixels: u32, pixel: u32) -> f32 {
    let world = pixel as f32 / map_pixels.max(1) as f32 - 0.5;
    rect.center().x + (world - camera_x) * scale
}

fn display_pixel(camera_x: f32, rect: Rect, scale: f32, map_pixels: u32, screen_x: f32) -> Option<u32> {
    if map_pixels == 0 {
        return None;
    }
    let world = camera_x + (screen_x - rect.center().x) / scale.max(1e-4);
    let pixel = ((world + 0.5) * map_pixels as f32).floor();
    if pixel < 0.0 || pixel >= map_pixels as f32 {
        None
    } else {
        Some(pixel as u32)
    }
}

fn original_at<P: MapPiece>(pieces: &[P], map_pixels: u32, display: u32) -> Option<usize> {
    if display >= map_pixels {
        return None;
    }
    let mut acc = 0u32;
    for piece in pieces {
        let next = acc + piece.pixels();
        if display < next {
            let original = piece.ref_at(display - acc);
            return (original >= 0).then_some(original as usize);
        }
        acc = next;
    }
    None
}

fn rgba(color: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

// ---------------------------------------------------------------------------
// Extensions section of the side menu.
// ---------------------------------------------------------------------------

pub fn section(ui: &mut egui::Ui, graphs: &mut [GraphSeries], tracks: &mut [Track]) {
    let title = if graphs.is_empty() {
        "Extensions".to_string()
    } else {
        format!("Extensions ({})", graphs.len())
    };
    ui.collapsing(title, |ui| {
        if graphs.is_empty() {
            for track in tracks.iter_mut() {
                let label = checkbox_label(&track.name);
                ui.checkbox(&mut track.on, label);
            }
            return;
        }
        let mut changed = None;
        for graph in graphs.iter_mut() {
            let before = graph.on;
            let label = checkbox_label(&graph.name);
            ui.checkbox(&mut graph.on, label);
            if graph.on != before {
                changed = Some((graph.name.clone(), graph.on));
            }
        }
        if let Some((name, on)) = changed {
            if let Some(track) = tracks.iter_mut().find(|track| track.name == name) {
                track.on = on;
            }
        }
    });
}

fn inflate_raw(src: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = DeflateDecoder::new(src);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|err| format!("could not inflate pretext graph ({err})"))?;
    Ok(out)
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let chunk = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "pretext file ended early".to_string())?;
    Ok(u32::from_le_bytes(chunk.try_into().unwrap()))
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    if text.is_empty() {
        "graph".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Span {
        start: u32,
        end: u32,
        coverage: f32,
    }

    impl ContigSpan for Span {
        fn pixel_range(&self) -> (u32, u32) {
            (self.start, self.end)
        }
        fn set_mean(&mut self, kind: TrackKind, mean: f32) {
            if kind == TrackKind::Coverage {
                self.coverage = mean;
            }
        }
    }

    fn block(name: &str, values: &[i32]) -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut raw = vec![0u8; 64 + values.len() * 4];
        let bytes = name.as_bytes();
        raw[..bytes.len().min(63)].copy_from_slice(&bytes[..bytes.len().min(63)]);
        for (i, value) in values.iter().enumerate() {
            raw[64 + i * 4..64 + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut out = b"psgh".to_vec();
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    #[test]
    fn custom_bedgraph_bigwig_and_alg_names_are_kept() {
        let mut bytes = block("coverage", &[0, 10, 20, 40]);
        bytes.extend(block("sample.bedgraph", &[0, 0, 8, 4]));
        bytes.extend(block("reads.bigWig", &[2, 2, 2, 4]));
        bytes.extend(block("contigs.alg", &[1, 0, 0, 0]));
        let mut contigs = [Span {
            start: 0,
            end: 4,
            coverage: 0.0,
        }];
        let parsed = read_graphs(&bytes, 0, 4, &mut contigs);
        assert_eq!(
            parsed.graphs.iter().map(|graph| graph.name.as_str()).collect::<Vec<_>>(),
            vec!["coverage", "sample.bedgraph", "reads.bigWig", "contigs.alg"]
        );
        assert!((parsed.graphs[0].values[3] - 1.0).abs() < 1e-5);
        assert!((parsed.graphs[1].values[2] - 1.0).abs() < 1e-5);
        assert!(parsed.tracks_on.contains(&TrackKind::Coverage));
        assert!(contigs[0].coverage > 0.0);
        assert!(kind_for("sample.bedgraph").is_none());
        assert!(kind_for("reads.bigWig").is_none());
        assert!(kind_for("contigs.alg").is_none());
        let series = series_from(parsed.graphs);
        assert!(series.iter().all(|graph| !graph.on));
    }
}
