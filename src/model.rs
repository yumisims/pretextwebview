//! Contact-map state for the egui PretextView shell.
//!
//! The map is a synthetic stand-in for a `.pretext` texture: each fragment
//! owns a span of the reference, and pixel intensity falls off with reference
//! distance. Reordering, cutting, and link-sorting change that picture the
//! same way the desktop viewer's edit list does.

use serde::{Deserialize, Serialize};

use crate::colormap::NAMES;

pub const MAP_PIXELS: u32 = 256;
pub const GENOME_BP: u64 = 48_000_000;
pub const TAGS: &[&str] = &[
    "Haplotig",
    "Unloc",
    "X",
    "Y",
    "Z",
    "W",
    "HAP1",
    "HAP2",
    "Target",
    "Contaminant",
    "Singleton",
    "X1",
    "X2",
    "Y1",
    "Y2",
    "Z1",
    "Z2",
    "W1",
    "W2",
    "I",
    "II",
    "III",
    "IV",
    "V",
    "U",
    "B1",
    "B2",
    "B3",
    "FalseDuplicate",
    "Primary",
    "vertPaint",
    "horzPaint",
    "crossPaint",
];

pub const PAINT_TAGS: &[&str] = &["vertPaint", "horzPaint", "crossPaint"];

pub const BACKGROUNDS: &[(&str, [u8; 4])] = &[
    ("Classic", [51, 153, 102, 255]),
    ("Jasper", [235, 162, 94, 255]),
    ("Heather", [158, 123, 181, 255]),
    ("Carolina", [74, 139, 168, 255]),
    ("White", [255, 255, 255, 255]),
    ("Grey", [195, 195, 195, 255]),
    ("Black", [0, 0, 0, 255]),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Normal,
    Edit,
    Waypoint,
    Scaffold,
    Meta,
    Extension,
    SelectSort,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "View",
            Self::Edit => "Edit",
            Self::Waypoint => "Waypoint",
            Self::Scaffold => "Scaffold",
            Self::Meta => "Meta tag",
            Self::Extension => "Extension",
            Self::SelectSort => "Select sort area",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {
    UnionFind,
    Fuse,
    DeepFuse,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::UnionFind => "UnionFind",
            Self::Fuse => "Fuse",
            Self::DeepFuse => "Deep Fuse",
        }
    }
}

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fragment {
    pub uid: u64,
    pub group: u64,
    pub name: String,
    pub source: usize,
    /// Reference coordinate of the low end of this piece, in map pixels.
    pub ref_lo: u32,
    pub pixels: u32,
    /// Length in base pairs. Zero means "derive it from the demo genome".
    #[serde(default)]
    pub bp: u64,
    pub inverted: bool,
    pub scaff_id: u32,
    pub meta: u64,
    pub coverage: f32,
    pub gap: f32,
    pub repeat: f32,
    pub telomere: f32,
    pub telomere_3: f32,
    pub telomere_5: f32,
    pub pixel_density: f32,
    pub break_score: f32,
    /// Display-local pixel where a density drop sits. Meaningful when `break_score` > 0.
    pub break_at: u32,
    pub cut: bool,
}

impl Fragment {
    pub fn display_name(&self) -> String {
        if self.cut {
            format!("{}:{}+{}", self.name, self.ref_lo, self.pixels)
        } else {
            self.name.clone()
        }
    }

    pub fn component_id(&self) -> String {
        self.display_name()
    }

    pub fn bp_len(&self) -> u64 {
        if self.bp > 0 {
            self.bp
        } else {
            (self.pixels as u64 * GENOME_BP) / MAP_PIXELS as u64
        }
    }

    pub fn ref_at(&self, local: u32) -> i32 {
        let local = local.min(self.pixels.saturating_sub(1));
        if !self.inverted {
            self.ref_lo as i32 + local as i32
        } else {
            self.ref_lo as i32 + self.pixels as i32 - 1 - local as i32
        }
    }

    pub fn left_ref(&self) -> i32 {
        self.ref_at(0)
    }

    pub fn right_ref(&self) -> i32 {
        self.ref_at(self.pixels.saturating_sub(1))
    }

    pub fn track_value(&self, kind: TrackKind) -> f32 {
        match kind {
            TrackKind::Coverage => self.coverage,
            TrackKind::Gap => self.gap,
            TrackKind::Repeat => self.repeat,
            TrackKind::Telomere => self.telomere,
            TrackKind::Telomere3 => self.telomere_3,
            TrackKind::Telomere5 => self.telomere_5,
            TrackKind::PixelDensity => self.pixel_density,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Waypoint {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditRecord {
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Clone)]
struct Hist {
    fragments: Vec<Fragment>,
    edits: Vec<EditRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedState {
    fragments: Vec<Fragment>,
    waypoints: Vec<Waypoint>,
    edits: Vec<EditRecord>,
    camera: Camera,
    gamma: [f32; 3],
    #[serde(default)]
    signal: crate::yumi::SignalMethod,
    colormap: usize,
    tags: Vec<String>,
    grey_tags: Vec<bool>,
    active_tag: usize,
    tracks_on: Vec<bool>,
    file_name: String,
}

pub struct Session {
    pub fragments: Vec<Fragment>,
    pristine: Vec<Fragment>,
    pub waypoints: Vec<Waypoint>,
    pub edits: Vec<EditRecord>,
    undo: Vec<Hist>,
    redo: Vec<Hist>,
    pub camera: Camera,
    pub gamma: [f32; 3],
    pub signal: crate::yumi::SignalMethod,
    pub colormap: usize,
    pub colormap_order: Vec<usize>,
    pub use_custom_colormap_order: bool,
    pub mode: Mode,
    pub grid_on: bool,
    pub grid_color: [u8; 4],
    pub grid_size: f32,
    pub tooltip_on: bool,
    pub tooltip_fg: [u8; 4],
    pub tooltip_bg: [u8; 4],
    pub tooltip_size: f32,
    pub id_bars_on: bool,
    pub id_bar_size: f32,
    pub labels_on: bool,
    pub label_fg: [u8; 4],
    pub label_bg: [u8; 4],
    pub label_size: f32,
    pub scale_on: bool,
    pub scale_fg: [u8; 4],
    pub scale_bg: [u8; 4],
    pub scale_size: f32,
    pub waypoints_always: bool,
    pub scaffolds_always: bool,
    pub meta_always: bool,
    pub invert_mouse: bool,
    pub active_tag: usize,
    pub grey_tags: Vec<bool>,
    pub tags: Vec<String>,
    pub tracks: Vec<Track>,
    pub file_name: String,
    pub background: usize,
    pub meta_outline: u8,
    pub meta_palette: u8,
    pub waypoint_base: [u8; 4],
    pub waypoint_selected: [u8; 4],
    pub waypoint_text: [u8; 4],
    pub waypoint_bg: [u8; 4],
    pub waypoint_size: f32,
    pub scaff_text: [u8; 4],
    pub scaff_bg: [u8; 4],
    pub scaff_size: f32,
    pub edit_select: [u8; 4],
    pub edit_edit: [u8; 4],
    pub edit_invert: [u8; 4],
    pub edit_text: [u8; 4],
    pub edit_bg: [u8; 4],
    pub meta_text: [u8; 4],
    pub meta_bg: [u8; 4],
    pub meta_size: f32,
    pub smallest_frag: u32,
    pub link_threshold: f32,
    pub sort_mode: SortMode,
    pub cut_threshold: f32,
    pub cut_window: u32,
    pub cut_smallest: u32,
    pub paint_scaff: u32,
    pub map_revision: u64,
    pub map_pixels: u32,
    pub genome_bp: u64,
    contacts: Option<crate::pretext::ContactMap>,
    next_uid: u64,
}

fn bin_center(bin: u32, side: u32, map: u32) -> u32 {
    let start = (bin as u64 * map as u64) / side.max(1) as u64;
    let end = ((bin as u64 + 1) * map as u64) / side.max(1) as u64;
    let end = end.max(start + 1);
    ((start + end - 1) / 2).min(map.saturating_sub(1) as u64) as u32
}

fn overview_index(mapped: u32, side: u32, full: u32) -> usize {
    if side == full {
        return mapped as usize;
    }
    ((mapped as u64 * full as u64) / side.max(1) as u64).min(full as u64 - 1) as usize
}

impl Session {
    /// Settings only. No contigs and no contact texture, so the window opens blank.
    pub fn empty() -> Self {
        let mut session = Self::demo();
        session.fragments.clear();
        session.pristine.clear();
        session.file_name.clear();
        session.map_pixels = 0;
        session.genome_bp = 0;
        session.map_revision = 0;
        session
    }

    pub fn demo() -> Self {
        let tags: Vec<String> = TAGS.iter().map(|s| (*s).to_string()).collect();
        let mut s = Self {
            fragments: Vec::new(),
            pristine: Vec::new(),
            waypoints: Vec::new(),
            edits: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            camera: Camera::default(),
            gamma: [0.0, 0.5, 1.0],
            signal: crate::yumi::SignalMethod::Pretext,
            colormap: crate::colormap::FALL,
            colormap_order: (0..NAMES.len()).collect(),
            use_custom_colormap_order: false,
            mode: Mode::Normal,
            grid_on: false,
            grid_color: [40, 40, 40, 180],
            grid_size: 1.0,
            tooltip_on: true,
            tooltip_fg: [255, 230, 120, 255],
            tooltip_bg: [32, 32, 32, 220],
            tooltip_size: 14.0,
            id_bars_on: true,
            id_bar_size: 4.0,
            labels_on: true,
            label_fg: [240, 185, 15, 255],
            label_bg: [145, 140, 115, 255],
            label_size: 16.0,
            scale_on: true,
            scale_fg: [220, 60, 60, 255],
            scale_bg: [32, 32, 32, 200],
            scale_size: 8.0,
            waypoints_always: true,
            scaffolds_always: true,
            meta_always: false,
            invert_mouse: false,
            active_tag: 0,
            grey_tags: vec![false; tags.len()],
            tags,
            tracks: default_tracks(),
            file_name: "demo.pretext".to_string(),
            background: 6,
            meta_outline: 1,
            meta_palette: 0,
            waypoint_base: [220, 40, 40, 255],
            waypoint_selected: [40, 90, 220, 255],
            waypoint_text: [255, 230, 120, 255],
            waypoint_bg: [40, 40, 40, 220],
            waypoint_size: 8.0,
            scaff_text: [255, 230, 120, 255],
            scaff_bg: [40, 40, 40, 200],
            scaff_size: 8.0,
            edit_select: [40, 180, 80, 255],
            edit_edit: [50, 110, 230, 255],
            edit_invert: [210, 50, 50, 255],
            edit_text: [255, 230, 120, 255],
            edit_bg: [40, 40, 40, 200],
            meta_text: [255, 230, 120, 255],
            meta_bg: [40, 40, 40, 200],
            meta_size: 12.0,
            smallest_frag: 2,
            link_threshold: 0.4,
            sort_mode: SortMode::UnionFind,
            cut_threshold: 0.05,
            cut_window: 8,
            cut_smallest: 8,
            paint_scaff: 1,
            map_revision: 1,
            map_pixels: MAP_PIXELS,
            genome_bp: GENOME_BP,
            contacts: None,
            next_uid: 1,
        };
        s.fragments = s.make_demo_fragments();
        // A fixed shuffle, so the opening map is a mis-ordered assembly.
        let order = [4, 0, 7, 2, 11, 5, 9, 1, 8, 3, 10, 6];
        let mut shuffled = Vec::with_capacity(order.len());
        for i in order {
            shuffled.push(s.fragments[i].clone());
        }
        s.fragments = shuffled;
        s.pristine = s.fragments.clone();
        s
    }

    fn make_demo_fragments(&mut self) -> Vec<Fragment> {
        let lengths = [28, 16, 22, 10, 30, 18, 14, 24, 12, 20, 26, 36];
        debug_assert_eq!(lengths.iter().sum::<u32>(), MAP_PIXELS);
        let mut ref_cursor = 0u32;
        let mut out = Vec::new();
        for (source, pixels) in lengths.into_iter().enumerate() {
            let uid = self.alloc_uid();
            // A reference gap between source 5 and 6 weakens one junction so
            // UnionFind at the default threshold leaves it, and Fuse joins it.
            if source == 6 {
                ref_cursor += 33;
            }
            let mut frag = Fragment {
                uid,
                group: uid,
                name: format!("ptg{source:06}l"),
                source,
                ref_lo: ref_cursor,
                pixels,
                bp: 0,
                inverted: false,
                scaff_id: if source < 4 {
                    1
                } else if source < 7 {
                    2
                } else {
                    0
                },
                meta: 0,
                coverage: 0.35 + 0.55 * ((source as f32) * 1.7).sin().mul_add(0.5, 0.5),
                gap: if source % 5 == 0 { 0.85 } else { 0.06 },
                repeat: (source as f32 * 0.37).fract(),
                telomere: if source == 0 || source + 1 == lengths.len() {
                    0.9
                } else {
                    0.08
                },
                telomere_3: if source + 1 == lengths.len() { 0.95 } else { 0.05 },
                telomere_5: if source == 0 { 0.95 } else { 0.05 },
                pixel_density: 0.45 + 0.4 * (source as f32 / lengths.len() as f32),
                break_score: 0.0,
                break_at: pixels / 2,
                cut: false,
            };
            if source == 2 {
                frag.meta |= 1 << 0; // Haplotig
            }
            if source == 5 {
                frag.meta |= 1 << 1; // Unloc
            }
            if source == 9 {
                frag.meta |= 1 << 9; // Contaminant
            }
            if source == 4 {
                frag.break_score = 0.20;
            }
            if source == 8 {
                frag.break_score = 0.08;
            }
            if source == 10 {
                frag.break_score = 0.02;
            }
            ref_cursor += pixels;
            out.push(frag);
        }
        out
    }

    fn alloc_uid(&mut self) -> u64 {
        let id = self.next_uid;
        self.next_uid += 1;
        id
    }

    pub fn total_pixels(&self) -> u32 {
        self.fragments.iter().map(|f| f.pixels).sum()
    }

    pub fn prefix(&self, index: usize) -> u32 {
        self.fragments[..index].iter().map(|f| f.pixels).sum()
    }

    pub fn fragment_at(&self, pixel: u32) -> Option<usize> {
        if pixel >= self.map_pixels {
            return None;
        }
        let mut acc = 0u32;
        for (i, frag) in self.fragments.iter().enumerate() {
            acc += frag.pixels;
            if pixel < acc {
                return Some(i);
            }
        }
        None
    }

    pub fn pix_index(&self) -> Vec<usize> {
        let mut v = Vec::with_capacity(self.map_pixels as usize);
        for (i, frag) in self.fragments.iter().enumerate() {
            for _ in 0..frag.pixels {
                v.push(i);
            }
        }
        v
    }

    pub fn is_grey(&self, index: usize) -> bool {
        let meta = self.fragments[index].meta;
        self.grey_tags.iter().enumerate().any(|(bit, on)| {
            *on && bit < 64 && (meta & (1u64 << bit)) != 0
        })
    }

    pub fn tag_names(&self, meta: u64) -> Vec<&str> {
        self.tags
            .iter()
            .enumerate()
            .filter(|(i, _)| meta & (1u64 << i) != 0)
            .map(|(_, name)| name.as_str())
            .collect()
    }

    pub fn summary_line(&self) -> String {
        let mb = self.genome_bp as f64 / 1_000_000.0;
        let px = self
            .contacts
            .as_ref()
            .map(|contacts| contacts.overview_side)
            .filter(|side| *side > 0)
            .unwrap_or(self.map_pixels);
        format!("{mb:.2} Mb · {} sequences · {px} px overview", self.fragments.len())
    }

    pub fn background_rgba(&self) -> [u8; 4] {
        BACKGROUNDS
            .get(self.background)
            .map(|(_, c)| *c)
            .unwrap_or([0, 0, 0, 255])
    }

    pub fn ordered_colormap(&self, slot: usize) -> usize {
        if self.use_custom_colormap_order {
            self.colormap_order.get(slot).copied().unwrap_or(slot)
        } else {
            slot
        }
    }

    pub fn bump(&mut self) {
        self.map_revision = self.map_revision.wrapping_add(1);
    }

    fn begin_edit(&mut self) {
        self.undo.push(Hist {
            fragments: self.fragments.clone(),
            edits: self.edits.clone(),
        });
        self.redo.clear();
        if self.undo.len() > 80 {
            self.undo.remove(0);
        }
    }

    pub fn undo_edit(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Hist {
            fragments: self.fragments.clone(),
            edits: self.edits.clone(),
        });
        self.fragments = prev.fragments;
        self.edits = prev.edits;
        self.bump();
        true
    }

    pub fn redo_edit(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Hist {
            fragments: self.fragments.clone(),
            edits: self.edits.clone(),
        });
        self.fragments = next.fragments;
        self.edits = next.edits;
        self.bump();
        true
    }

    pub fn redo_all(&mut self) -> usize {
        let mut n = 0;
        while self.redo_edit() {
            n += 1;
        }
        n
    }

    pub fn erase_all(&mut self) {
        self.begin_edit();
        self.fragments = self.pristine.clone();
        self.edits.clear();
        self.waypoints.clear();
        self.bump();
        self.edits.push(EditRecord {
            summary: "erase all edits".to_string(),
        });
    }

    /// Split wherever `pixel` falls strictly inside a fragment.
    fn split_at_pixel(&mut self, pixel: u32) -> bool {
        let mut acc = 0u32;
        for i in 0..self.fragments.len() {
            let end = acc + self.fragments[i].pixels;
            if pixel > acc && pixel < end {
                let local = pixel - acc;
                self.split_index(i, local);
                return true;
            }
            if pixel <= end {
                return false;
            }
            acc = end;
        }
        false
    }

    fn split_index(&mut self, index: usize, local: u32) {
        let left_pixels = local;
        let mut right = self.fragments[index].clone();
        right.uid = self.alloc_uid();
        right.cut = true;
        self.fragments[index].cut = true;
        let inverted = self.fragments[index].inverted;
        let old_pixels = self.fragments[index].pixels;
        let old_bp = self.fragments[index].bp;
        let old_break = self.fragments[index].break_at;
        let old_break_score = self.fragments[index].break_score;
        if !inverted {
            right.ref_lo = self.fragments[index].ref_lo + left_pixels;
            right.pixels = old_pixels - left_pixels;
            self.fragments[index].pixels = left_pixels;
        } else {
            // Screen-left is the high reference end.
            let left_ref = self.fragments[index].ref_lo + (old_pixels - left_pixels);
            right.ref_lo = self.fragments[index].ref_lo;
            right.pixels = old_pixels - left_pixels;
            self.fragments[index].ref_lo = left_ref;
            self.fragments[index].pixels = left_pixels;
        }
        if old_bp > 0 && old_pixels > 0 {
            let left_bp = old_bp * left_pixels as u64 / old_pixels as u64;
            self.fragments[index].bp = left_bp.max(1);
            right.bp = (old_bp - left_bp).max(1);
        }
        if old_break_score > 0.0 && old_break >= left_pixels {
            right.break_score = old_break_score;
            right.break_at = old_break - left_pixels;
            self.fragments[index].break_score = 0.0;
        } else {
            right.break_score = 0.0;
        }
        self.fragments.insert(index + 1, right);
    }

    pub fn isolate_range(&mut self, mut start: u32, mut end: u32) -> Vec<u64> {
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        start = start.min(self.map_pixels);
        end = end.min(self.map_pixels);
        if start == end {
            return Vec::new();
        }
        let will_split = self.pixel_inside(start) || self.pixel_inside(end);
        if will_split {
            self.begin_edit();
        }
        self.split_at_pixel(start);
        self.split_at_pixel(end);
        let mut uids = Vec::new();
        let mut acc = 0u32;
        for frag in &self.fragments {
            let next = acc + frag.pixels;
            if acc >= start && next <= end {
                uids.push(frag.uid);
            }
            acc = next;
        }
        if will_split {
            self.edits.push(EditRecord {
                summary: format!("split edges of {start}..{end}"),
            });
            self.bump();
        }
        uids
    }

    pub fn split_pixel(&mut self, pixel: u32) -> bool {
        if !self.pixel_inside(pixel) {
            return false;
        }
        self.begin_edit();
        let split = self.split_at_pixel(pixel);
        if split {
            self.edits.push(EditRecord {
                summary: format!("split at pixel {pixel}"),
            });
            self.bump();
        } else {
            self.undo.pop();
        }
        split
    }

    fn pixel_inside(&self, pixel: u32) -> bool {
        let mut acc = 0u32;
        for frag in &self.fragments {
            if pixel > acc && pixel < acc + frag.pixels {
                return true;
            }
            acc += frag.pixels;
        }
        false
    }

    /// Remember the map so a grab can move more than once and still undo as one edit.
    pub fn arm_edit(&mut self) {
        self.begin_edit();
    }

    /// Drop an armed edit that never changed the map.
    pub fn disarm_edit(&mut self) {
        self.undo.pop();
    }

    pub fn note_edit(&mut self, summary: impl Into<String>) {
        self.edits.push(EditRecord {
            summary: summary.into(),
        });
    }

    pub fn place_uids(&mut self, uids: &[u64], dest_px: u32) -> Result<(), &'static str> {
        self.relocate(uids, dest_px, true)
    }

    /// Move a block that was already stored with [`Self::arm_edit`].
    pub fn place_armed(&mut self, uids: &[u64], dest_px: u32) -> Result<(), &'static str> {
        self.relocate(uids, dest_px, false)
    }

    fn relocate(&mut self, uids: &[u64], dest_px: u32, record: bool) -> Result<(), &'static str> {
        if uids.is_empty() {
            return Err("nothing selected");
        }
        let mut idxs = Vec::new();
        for uid in uids {
            let i = self
                .fragments
                .iter()
                .position(|f| f.uid == *uid)
                .ok_or("selection is stale")?;
            idxs.push(i);
        }
        idxs.sort_unstable();
        if idxs.windows(2).any(|w| w[1] != w[0] + 1) {
            return Err("selection is not contiguous");
        }
        let start_i = idxs[0];
        let end_i = *idxs.last().unwrap();
        let start_px = self.prefix(start_i);
        let removed = self.prefix(end_i) + self.fragments[end_i].pixels - start_px;
        if dest_px >= start_px && dest_px < start_px + removed {
            return Err("place the selection outside itself");
        }
        if record {
            self.begin_edit();
        }
        let moving: Vec<_> = self.fragments.drain(start_i..=end_i).collect();
        let dest = if dest_px >= start_px + removed {
            dest_px - removed
        } else {
            dest_px
        };
        self.split_at_pixel(dest);
        let insert_at = self.index_at_boundary(dest);
        let summary = format!("moved {} fragment(s) to pixel {dest}", moving.len());
        self.fragments.splice(insert_at..insert_at, moving);
        if record {
            self.edits.push(EditRecord { summary });
        }
        self.bump();
        debug_assert_eq!(self.total_pixels(), self.map_pixels);
        Ok(())
    }

    fn index_at_boundary(&self, pixel: u32) -> usize {
        let mut acc = 0u32;
        for (i, frag) in self.fragments.iter().enumerate() {
            if acc == pixel {
                return i;
            }
            acc += frag.pixels;
        }
        self.fragments.len()
    }

    pub fn invert_uids(&mut self, uids: &[u64]) -> Result<(), &'static str> {
        if uids.is_empty() {
            return Err("nothing selected");
        }
        let mut idxs = Vec::new();
        for uid in uids {
            idxs.push(
                self.fragments
                    .iter()
                    .position(|f| f.uid == *uid)
                    .ok_or("selection is stale")?,
            );
        }
        idxs.sort_unstable();
        if idxs.windows(2).any(|w| w[1] != w[0] + 1) {
            return Err("selection is not contiguous");
        }
        self.begin_edit();
        let start = idxs[0];
        let end = *idxs.last().unwrap();
        self.fragments[start..=end].reverse();
        for frag in &mut self.fragments[start..=end] {
            frag.inverted = !frag.inverted;
        }
        self.edits.push(EditRecord {
            summary: format!("inverted {} fragment(s)", end - start + 1),
        });
        self.bump();
        Ok(())
    }

    pub fn toggle_tag(&mut self, index: usize) {
        if index >= self.fragments.len() || self.active_tag >= 64 {
            return;
        }
        self.begin_edit();
        let bit = 1u64 << self.active_tag;
        let name = self.tags.get(self.active_tag).cloned().unwrap_or_default();
        let frag_name = self.fragments[index].display_name();
        if self.fragments[index].meta & bit == 0 {
            self.fragments[index].meta |= bit;
            self.edits.push(EditRecord {
                summary: format!("tagged {frag_name} with {name}"),
            });
        } else {
            self.fragments[index].meta &= !bit;
            self.edits.push(EditRecord {
                summary: format!("cleared {name} on {frag_name}"),
            });
        }
        self.bump();
    }

    pub fn set_scaffold(&mut self, index: usize, scaff: u32) {
        if index >= self.fragments.len() {
            return;
        }
        if self.fragments[index].scaff_id == scaff {
            return;
        }
        self.begin_edit();
        let name = self.fragments[index].display_name();
        self.fragments[index].scaff_id = scaff;
        self.edits.push(EditRecord {
            summary: format!("painted {name} as scaffold {scaff}"),
        });
        self.bump();
    }

    pub fn move_tags_to_end(&mut self, mask: u64) -> usize {
        if mask == 0 {
            return 0;
        }
        self.begin_edit();
        let mut keep = Vec::new();
        let mut tail = Vec::new();
        for frag in self.fragments.drain(..) {
            if frag.meta & mask != 0 {
                tail.push(frag);
            } else {
                keep.push(frag);
            }
        }
        let n = tail.len();
        keep.append(&mut tail);
        self.fragments = keep;
        self.edits.push(EditRecord {
            summary: format!("moved {n} tagged fragment(s) to the end"),
        });
        self.bump();
        n
    }

    pub fn pixel_cut(&mut self, only: Option<&[u64]>) -> usize {
        let targets: Vec<(u64, u32)> = self
            .fragments
            .iter()
            .filter_map(|frag| {
                if frag.break_score <= 0.0 || frag.break_score < self.cut_threshold {
                    return None;
                }
                if frag.pixels < self.cut_smallest.saturating_mul(2) {
                    return None;
                }
                if let Some(ids) = only {
                    if !ids.contains(&frag.uid) {
                        return None;
                    }
                }
                let margin = self.cut_window.min(frag.pixels / 2);
                let at = frag.break_at.clamp(margin, frag.pixels - margin);
                if at < self.cut_smallest || frag.pixels - at < self.cut_smallest {
                    return None;
                }
                Some((frag.uid, at))
            })
            .collect();
        if targets.is_empty() {
            return 0;
        }
        self.begin_edit();
        let mut cuts = 0;
        for (uid, local) in targets {
            if let Some(i) = self.fragments.iter().position(|f| f.uid == uid) {
                let pixel = self.prefix(i) + local;
                if self.split_at_pixel(pixel) {
                    cuts += 1;
                }
            }
        }
        self.edits.push(EditRecord {
            summary: format!("pixel cut ({cuts})"),
        });
        self.bump();
        debug_assert_eq!(self.total_pixels(), self.map_pixels);
        cuts
    }

    pub fn glue_adjacent(&mut self, only: Option<&[u64]>) -> usize {
        let can = |a: &Fragment, b: &Fragment| {
            if a.group != b.group || a.inverted != b.inverted {
                return false;
            }
            if !a.inverted {
                a.ref_lo + a.pixels == b.ref_lo
            } else {
                b.ref_lo + b.pixels == a.ref_lo
            }
        };
        let mut glued = 0usize;
        let mut i = 0;
        let mut started = false;
        while i + 1 < self.fragments.len() {
            let selected = only
                .map(|ids| ids.contains(&self.fragments[i].uid) && ids.contains(&self.fragments[i + 1].uid))
                .unwrap_or(true);
            if selected && can(&self.fragments[i], &self.fragments[i + 1]) {
                if !started {
                    self.begin_edit();
                    started = true;
                }
                let extra = self.fragments[i + 1].pixels;
                let right_ref = self.fragments[i + 1].ref_lo;
                if self.fragments[i].inverted {
                    self.fragments[i].ref_lo = right_ref;
                }
                let extra_bp = self.fragments[i + 1].bp;
                self.fragments[i].pixels += extra;
                if self.fragments[i].bp > 0 || extra_bp > 0 {
                    self.fragments[i].bp += extra_bp;
                }
                if self.fragments[i].break_score <= 0.0 {
                    self.fragments[i].break_score = self.fragments[i + 1].break_score;
                    self.fragments[i].break_at = self.fragments[i].pixels - extra + self.fragments[i + 1].break_at;
                }
                self.fragments.remove(i + 1);
                glued += 1;
            } else {
                i += 1;
            }
        }
        if glued > 0 {
            self.edits.push(EditRecord {
                summary: format!("glued {glued} join(s)"),
            });
            self.bump();
        }
        glued
    }

    pub fn sort_fragments(&mut self, only_slots: Option<&[usize]>) -> usize {
        let slots: Vec<usize> = match only_slots {
            Some(slots) if slots.len() >= 3 => {
                let mut s = slots.to_vec();
                s.sort_unstable();
                s.dedup();
                s
            }
            Some(_) => return 0,
            None => (0..self.fragments.len()).collect(),
        };
        if slots.len() < 2 {
            return 0;
        }
        let contiguous = slots.windows(2).all(|w| w[1] == w[0] + 1);
        let (work_slots, anchor_left, anchor_right) = if contiguous && only_slots.is_some() && slots.len() >= 3 {
            (slots[1..slots.len() - 1].to_vec(), true, true)
        } else {
            (slots.clone(), false, false)
        };
        if work_slots.len() < 2 && anchor_left {
            return 0;
        }
        let items: Vec<Fragment> = work_slots.iter().map(|i| self.fragments[*i].clone()).collect();
        let (sorted, links) = link_sort(items, self.link_threshold, self.smallest_frag, self.sort_mode);
        self.begin_edit();
        for (slot, frag) in work_slots.into_iter().zip(sorted) {
            self.fragments[slot] = frag;
        }
        let _ = (anchor_left, anchor_right);
        let where_ = if only_slots.is_some() { "local" } else { "global" };
        self.edits.push(EditRecord {
            summary: format!(
                "pixel sort {where_} {} ({links} links)",
                self.sort_mode.label()
            ),
        });
        self.bump();
        links
    }

    pub fn add_waypoint(&mut self, x: f32, y: f32) {
        self.waypoints.push(Waypoint {
            x,
            y,
            zoom: self.camera.zoom,
        });
    }

    pub fn remove_waypoint_near(&mut self, x: f32, y: f32) -> bool {
        let mut best: Option<(usize, f32)> = None;
        for (i, wp) in self.waypoints.iter().enumerate() {
            let d = (wp.x - x).hypot(wp.y - y);
            if d < 0.03 && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        if let Some((i, _)) = best {
            self.waypoints.remove(i);
            true
        } else {
            false
        }
    }

    pub fn jump_waypoint(&mut self, index: usize) {
        if let Some(wp) = self.waypoints.get(index) {
            self.camera.x = wp.x;
            self.camera.y = wp.y;
            self.camera.zoom = wp.zoom.max(0.25);
        }
    }

    pub fn focus_fragment(&mut self, index: usize) {
        if index >= self.fragments.len() {
            return;
        }
        let start = self.prefix(index);
        let end = start + self.fragments[index].pixels;
        self.zoom_to_pixels(start, end);
    }

    pub fn focus_scaffold(&mut self, scaff: u32) {
        let mut start = None;
        let mut end = 0u32;
        let mut acc = 0u32;
        for frag in &self.fragments {
            let next = acc + frag.pixels;
            if frag.scaff_id == scaff {
                start.get_or_insert(acc);
                end = next;
            }
            acc = next;
        }
        if let Some(start) = start {
            self.zoom_to_pixels(start, end);
        }
    }

    pub fn zoom_to_pixels(&mut self, start: u32, end: u32) {
        let a = start.min(end);
        let b = start.max(end).max(a + 1);
        let mid = (a + b) as f32 / 2.0 / self.map_pixels as f32 - 0.5;
        let span = (b - a) as f32 / self.map_pixels as f32;
        self.camera.x = mid;
        self.camera.y = mid;
        let mut zoom = (1.0 / span).clamp(0.25, 80.0);
        if let Some(contacts) = &self.contacts {
            // A deeper zoom decodes individual tiles on the UI thread. Keep
            // list-focus and similar jumps on the overview image.
            let overview = contacts.overview_side.max(1) as f32;
            zoom = zoom.min((1.8 * overview / 720.0).max(1.0));
        }
        self.camera.zoom = zoom;
    }

    pub fn world_to_pixel(&self, world: f32) -> i32 {
        ((world + 0.5) * self.map_pixels as f32).floor() as i32
    }

    pub fn pixel_to_world_center(&self, pixel: u32) -> f32 {
        (pixel as f32 + 0.5) / self.map_pixels.max(1) as f32 - 0.5
    }

    pub fn wants_detail(&self, view_min: f32) -> bool {
        let Some(contacts) = &self.contacts else {
            return false;
        };
        if contacts.overview_side == 0 || contacts.overview_mip == 0 || !contacts.has_source() {
            return false;
        }
        let screen_per_texel = view_min * self.camera.zoom / contacts.overview_side as f32;
        screen_per_texel > 3.0
    }

    /// Suggested AGP download name: the map file without its `.pretext` suffix.
    pub fn agp_file_name(&self) -> String {
        agp_export_name(&self.file_name)
    }

    pub fn to_agp(&self) -> String {
        let mut lines = vec!["##agp-version 2.1".to_string()];
        let mut objects: Vec<(String, Vec<usize>)> = Vec::new();
        let mut current_scaff = None;
        let mut bucket = Vec::new();
        for (i, frag) in self.fragments.iter().enumerate() {
            if frag.scaff_id == 0 {
                if !bucket.is_empty() {
                    if let Some(id) = current_scaff.take() {
                        objects.push((format!("scaffold_{id}"), std::mem::take(&mut bucket)));
                    }
                }
                objects.push((frag.component_id(), vec![i]));
                continue;
            }
            if current_scaff != Some(frag.scaff_id) {
                if !bucket.is_empty() {
                    if let Some(id) = current_scaff.take() {
                        objects.push((format!("scaffold_{id}"), std::mem::take(&mut bucket)));
                    }
                }
                current_scaff = Some(frag.scaff_id);
            }
            bucket.push(i);
        }
        if !bucket.is_empty() {
            if let Some(id) = current_scaff {
                objects.push((format!("scaffold_{id}"), bucket));
            }
        }
        for (object, idxs) in objects {
            let mut cursor = 1u64;
            for (part, index) in idxs.into_iter().enumerate() {
                let frag = &self.fragments[index];
                let len = frag.bp_len().max(1);
                let end = cursor + len - 1;
                let strand = if frag.inverted { '-' } else { '+' };
                lines.push(format!(
                    "{object}\t{cursor}\t{end}\t{}\tW\t{}\t1\t{len}\t{strand}",
                    part + 1,
                    frag.component_id()
                ));
                cursor = end + 1;
            }
        }
        lines.join("\n") + "\n"
    }

    pub fn load_agp(&mut self, text: &str) -> Result<usize, String> {
        let mut wanted: Vec<(String, bool)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 9 || cols[4] != "W" {
                continue;
            }
            wanted.push((cols[5].to_string(), cols[8] == "-"));
        }
        if wanted.is_empty() {
            return Err("no W lines in that AGP".to_string());
        }
        self.begin_edit();
        let mut pool = std::mem::take(&mut self.fragments);
        let mut next = Vec::new();
        for (name, inverted) in &wanted {
            if let Some(pos) = pool.iter().position(|f| f.component_id() == *name || f.name == *name) {
                let mut frag = pool.remove(pos);
                frag.inverted = *inverted;
                next.push(frag);
            }
        }
        let placed = next.len();
        next.append(&mut pool);
        self.fragments = next;
        self.edits.push(EditRecord {
            summary: format!("loaded AGP ({placed} parts)"),
        });
        self.bump();
        Ok(placed)
    }

    pub fn save_json(&self) -> String {
        let state = SavedState {
            fragments: self.fragments.clone(),
            waypoints: self.waypoints.clone(),
            edits: self.edits.clone(),
            camera: self.camera,
            gamma: self.gamma,
            signal: self.signal,
            colormap: self.colormap,
            tags: self.tags.clone(),
            grey_tags: self.grey_tags.clone(),
            active_tag: self.active_tag,
            tracks_on: self.tracks.iter().map(|t| t.on).collect(),
            file_name: self.file_name.clone(),
        };
        serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn load_json(&mut self, text: &str) -> Result<(), String> {
        let state: SavedState = serde_json::from_str(text).map_err(|err| err.to_string())?;
        if state.fragments.iter().map(|f| f.pixels).sum::<u32>() != self.map_pixels {
            return Err("state does not cover the demo map".to_string());
        }
        self.fragments = state.fragments;
        self.waypoints = state.waypoints;
        self.edits = state.edits;
        self.camera = state.camera;
        self.gamma = state.gamma;
        self.signal = state.signal;
        self.colormap = state.colormap;
        if !state.tags.is_empty() {
            self.tags = state.tags;
        }
        if !state.grey_tags.is_empty() {
            self.grey_tags = state.grey_tags;
        }
        self.active_tag = state.active_tag;
        for (track, on) in self.tracks.iter_mut().zip(state.tracks_on) {
            track.on = on;
        }
        self.file_name = state.file_name;
        self.next_uid = self.fragments.iter().map(|f| f.uid).max().unwrap_or(0) + 1;
        self.undo.clear();
        self.redo.clear();
        self.bump();
        Ok(())
    }

    pub fn edits_text(&self) -> String {
        if self.edits.is_empty() {
            return "No edits.\n".to_string();
        }
        self.edits
            .iter()
            .enumerate()
            .map(|(i, edit)| format!("{}. {}", i + 1, edit.summary))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    pub fn install_map(&mut self, loaded: crate::pretext::LoadedMap, file_name: String) -> String {
        self.genome_bp = loaded.genome_bp;
        self.map_pixels = loaded.map_pixels;
        self.next_uid = 1;
        self.fragments = loaded
            .contigs
            .into_iter()
            .enumerate()
            .map(|(source, contig)| {
                let uid = self.alloc_uid();
                Fragment {
                    uid,
                    group: uid,
                    name: contig.name,
                    source,
                    ref_lo: contig.ref_lo,
                    pixels: contig.pixels,
                    bp: contig.bp,
                    inverted: false,
                    scaff_id: 0,
                    meta: 0,
                    coverage: contig.coverage,
                    gap: contig.gap,
                    repeat: contig.repeat,
                    telomere: contig.telomere,
                    telomere_3: contig.telomere_3,
                    telomere_5: contig.telomere_5,
                    pixel_density: contig.pixel_density,
                    break_score: 0.0,
                    break_at: contig.pixels / 2,
                    cut: false,
                }
            })
            .collect();
        self.pristine = self.fragments.clone();
        self.waypoints.clear();
        self.edits.clear();
        self.undo.clear();
        self.redo.clear();
        self.camera = Camera::default();
        for track in &mut self.tracks {
            if loaded.tracks_on.contains(&track.kind) {
                track.on = true;
            }
        }
        let side = loaded.contacts.overview_side;
        self.contacts = Some(loaded.contacts);
        self.file_name = file_name;
        self.mode = Mode::Edit;
        self.bump();
        let mb = self.genome_bp as f64 / 1_000_000.0;
        let mut message = format!(
            "Loaded {} — {} sequences, {mb:.2} Mb, {side}×{side} contact map",
            self.file_name,
            self.fragments.len()
        );
        if loaded.omitted > 0 {
            message.push_str(&format!(
                ", {} sequences shorter than one pixel share a neighbour's bin",
                loaded.omitted
            ));
        }
        if loaded.failed_tiles > 0 {
            message.push_str(&format!(", {} tiles could not be decoded", loaded.failed_tiles));
        }
        message.push_str(". Scroll to zoom; drag with the pan button to move the view.");
        message
    }

    /// Swap in the decoded tile source and graph tracks without resetting the camera.
    pub fn complete_map(&mut self, loaded: crate::pretext::LoadedMap) -> String {
        for (frag, contig) in self.fragments.iter_mut().zip(&loaded.contigs) {
            frag.coverage = contig.coverage;
            frag.gap = contig.gap;
            frag.repeat = contig.repeat;
            frag.telomere = contig.telomere;
            frag.telomere_3 = contig.telomere_3;
            frag.telomere_5 = contig.telomere_5;
            frag.pixel_density = contig.pixel_density;
        }
        for track in &mut self.tracks {
            if loaded.tracks_on.contains(&track.kind) {
                track.on = true;
            }
        }
        let side = loaded.contacts.overview_side;
        let failed = loaded.failed_tiles;
        self.contacts = Some(loaded.contacts);
        self.bump();
        let mb = self.genome_bp as f64 / 1_000_000.0;
        let mut message = format!(
            "Loaded {} — {} sequences, {mb:.2} Mb, {side}×{side} contact map",
            self.file_name,
            self.fragments.len()
        );
        if failed > 0 {
            message.push_str(&format!(", {failed} tiles could not be decoded"));
        }
        message.push_str(". Scroll to zoom; drag with the pan button to move the view.");
        message
    }

    /// Display-order contact grid used by the suggestion scan. Real maps use the
    /// overview texture. The synthetic demo uses the same reference-distance curve
    /// as [`contact_at`].
    pub(crate) fn contact_grid(&self) -> crate::suggest::ContactGrid {
        use crate::suggest::ContactGrid;
        let side = if let Some(contacts) = &self.contacts {
            let overview = contacts.overview_side;
            if overview == 0 || self.map_pixels == 0 || self.fragments.is_empty() {
                0
            } else {
                overview.min(384)
            }
        } else if self.map_pixels == 0 || self.fragments.is_empty() {
            0
        } else {
            self.map_pixels.min(384)
        };
        if side == 0 {
            return ContactGrid {
                side: 0,
                values: Vec::new(),
                owner: Vec::new(),
                pixel: Vec::new(),
            };
        }
        let prefix = self.prefix_sums();
        let map = self.map_pixels.max(1);
        let mut orig = vec![0u32; side as usize];
        let mut owner = vec![0usize; side as usize];
        let mut pixel = vec![0u32; side as usize];
        for bin in 0..side {
            let display = bin_center(bin, side, map);
            let (original, index) = self.original_pixel(display as i32, &prefix);
            orig[bin as usize] = original;
            owner[bin as usize] = index;
            pixel[bin as usize] = display;
        }
        let index = if self.contacts.is_none() {
            self.pix_index()
        } else {
            Vec::new()
        };
        let mut values = vec![0.0f32; (side as usize) * (side as usize)];
        for y in 0..side {
            for x in 0..side {
                let value = if let Some(contacts) = &self.contacts {
                    contacts.overview_at(orig[x as usize], orig[y as usize]) as f32 / 255.0
                } else {
                    contact_at(self, pixel[x as usize], pixel[y as usize], &index)
                };
                values[(y * side + x) as usize] = value;
            }
        }
        ContactGrid { side, values, owner, pixel }
    }

    /// RGBA image of the whole map. `max_side` caps the contact overview so a
    /// load can refresh a small preview without uploading a 2048² texture.
    pub fn raster_rgba(&self, max_side: u32) -> (u32, Vec<u8>) {
        if self.contacts.is_some() {
            self.raster_overview(max_side)
        } else {
            let rgba = self.raster_synthetic();
            (self.map_pixels.max(1), rgba)
        }
    }

    pub fn raster_window(&self, mut x0: f32, mut y0: f32, mut x1: f32, mut y1: f32, samples: u32) -> Vec<u8> {
        let samples = samples.max(2);
        x0 = x0.min(x1);
        y0 = y0.min(y1);
        x1 = x1.max(x0 + 1e-4);
        y1 = y1.max(y0 + 1e-4);
        let prefix = self.prefix_sums();
        let span = ((x1 - x0).max(y1 - y0) * self.map_pixels.max(1) as f32).max(1.0);
        let step = span / samples as f32;
        let mip = if step <= 1.0 {
            0
        } else {
            step.log2().floor().max(0.0) as u32
        };
        let mut rgba = vec![0u8; (samples as usize) * (samples as usize) * 4];
        for row in 0..samples {
            let world_y = y0 + (y1 - y0) * (row as f32 + 0.5) / samples as f32;
            let display_y = self.world_to_pixel(world_y);
            let (orig_y, iy) = self.original_pixel(display_y, &prefix);
            for col in 0..samples {
                let world_x = x0 + (x1 - x0) * (col as f32 + 0.5) / samples as f32;
                let display_x = self.world_to_pixel(world_x);
                let (orig_x, ix) = self.original_pixel(display_x, &prefix);
                let mut value = self
                    .contacts
                    .as_ref()
                    .map(|contacts| contacts.sample(orig_x, orig_y, mip) as f32 / 255.0)
                    .unwrap_or(0.0);
                if self.is_grey(ix) || self.is_grey(iy) {
                    value *= 0.2;
                }
                let px = self.color_of(value);
                let offset = ((row * samples + col) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&px);
            }
        }
        rgba
    }

    fn raster_synthetic(&self) -> Vec<u8> {
        let index = self.pix_index();
        let side = self.map_pixels;
        let mut rgba = vec![0u8; side as usize * side as usize * 4];
        for y in 0..side {
            for x in 0..side {
                let mut value = contact_at(self, x, y, &index);
                let ix = index.get(x as usize).copied().unwrap_or(0);
                let iy = index.get(y as usize).copied().unwrap_or(0);
                if self.is_grey(ix) || self.is_grey(iy) {
                    value *= 0.2;
                }
                let px = self.color_of(value);
                let offset = ((y * side + x) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&px);
            }
        }
        rgba
    }

    fn raster_overview(&self, max_side: u32) -> (u32, Vec<u8>) {
        let Some(contacts) = &self.contacts else {
            return (1, vec![0, 0, 0, 255]);
        };
        let full = contacts.overview_side.max(1);
        let side = full.min(max_side.max(1));
        let map_x = self.display_to_original_overview(side);
        let owners = self.overview_owners(side);
        let overview = contacts.overview.lock().unwrap_or_else(|err| err.into_inner());
        let mut rgba = vec![0u8; side as usize * side as usize * 4];
        for y in 0..side {
            let oy = overview_index(map_x[y as usize], side, full);
            let iy = owners[y as usize];
            for x in 0..side {
                let ox = overview_index(map_x[x as usize], side, full);
                let mut value = overview.get(oy * full as usize + ox).copied().unwrap_or(0) as f32 / 255.0;
                if self.is_grey(owners[x as usize]) || self.is_grey(iy) {
                    value *= 0.2;
                }
                let px = self.color_of(value);
                let offset = ((y * side + x) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&px);
            }
        }
        (side, rgba)
    }

    fn color_of(&self, value: f32) -> [u8; 4] {
        let curved = match self.signal {
            crate::yumi::SignalMethod::Pretext => crate::colormap::tone(value, self.gamma),
            crate::yumi::SignalMethod::Yumi => crate::yumi::tone(value, self.gamma),
        };
        crate::colormap::sample(self.colormap, curved)
    }

    fn prefix_sums(&self) -> Vec<u32> {
        let mut prefix = Vec::with_capacity(self.fragments.len() + 1);
        prefix.push(0);
        for frag in &self.fragments {
            prefix.push(prefix.last().copied().unwrap_or(0u32).saturating_add(frag.pixels));
        }
        prefix
    }

    fn original_pixel(&self, display: i32, prefix: &[u32]) -> (u32, usize) {
        if self.fragments.is_empty() || self.map_pixels == 0 {
            return (0, 0);
        }
        let last = self.fragments.len() - 1;
        if display <= 0 {
            let orig = self.fragments[0].ref_at(0).clamp(0, self.map_pixels as i32 - 1) as u32;
            return (orig, 0);
        }
        let display = (display as u32).min(self.map_pixels - 1);
        let index = prefix.partition_point(|&start| start <= display).saturating_sub(1).min(last);
        let start = prefix.get(index).copied().unwrap_or(0);
        let frag = &self.fragments[index];
        let local = display.saturating_sub(start).min(frag.pixels.saturating_sub(1));
        let orig = frag.ref_at(local).clamp(0, self.map_pixels as i32 - 1) as u32;
        (orig, index)
    }

    fn display_to_original_overview(&self, side: u32) -> Vec<u32> {
        let side_us = side.max(1) as usize;
        let map = self.map_pixels.max(1) as u64;
        let mut out = vec![0u32; side_us];
        let mut acc = 0u32;
        for (i, frag) in self.fragments.iter().enumerate() {
            let start = (acc as u64 * side_us as u64 / map) as usize;
            let next_acc = acc.saturating_add(frag.pixels);
            let end = if i + 1 == self.fragments.len() {
                side_us
            } else {
                (next_acc as u64 * side_us as u64 / map) as usize
            };
            let span = end.saturating_sub(start).max(1);
            for (k, ov) in (start..end.min(side_us)).enumerate() {
                let local = ((k as u64 * frag.pixels as u64) / span as u64).min(frag.pixels.saturating_sub(1) as u64) as u32;
                let orig = frag.ref_at(local).clamp(0, self.map_pixels as i32 - 1) as u32;
                out[ov] = ((orig as u64 * side_us as u64) / map).min(side_us as u64 - 1) as u32;
            }
            acc = next_acc;
        }
        out
    }

    fn overview_owners(&self, side: u32) -> Vec<usize> {
        let side_us = side.max(1) as usize;
        let map = self.map_pixels.max(1) as u64;
        let mut owners = vec![0usize; side_us];
        let mut acc = 0u32;
        for (i, frag) in self.fragments.iter().enumerate() {
            let start = (acc as u64 * side_us as u64 / map) as usize;
            acc = acc.saturating_add(frag.pixels);
            let end = if i + 1 == self.fragments.len() {
                side_us
            } else {
                (acc as u64 * side_us as u64 / map) as usize
            };
            for px in start..end.min(side_us) {
                owners[px] = i;
            }
        }
        owners
    }

    pub fn selection_clipboard(&self, uids: &[u64]) -> String {
        let mut lines = Vec::new();
        for uid in uids {
            if let Some(frag) = self.fragments.iter().find(|f| f.uid == *uid) {
                let mb = frag.bp_len() as f64 / 1_000_000.0;
                lines.push(format!("{}  0.00–{mb:.2} Mb", frag.display_name()));
            }
        }
        lines.join("\n")
    }

    pub fn reset_profile(&mut self) {
        let fresh = Self::demo();
        self.invert_mouse = false;
        self.background = fresh.background;
        self.meta_outline = fresh.meta_outline;
        self.meta_palette = fresh.meta_palette;
        self.colormap_order = fresh.colormap_order;
        self.use_custom_colormap_order = false;
        self.tracks = fresh.tracks;
        self.gamma = [0.0, 0.5, 1.0];
        self.signal = crate::yumi::SignalMethod::Pretext;
        self.bump();
    }

    pub fn consolidate(&mut self, uids: &[u64]) -> Result<Vec<u64>, &'static str> {
        if uids.len() < 2 {
            return Err("mark at least two contigs with Tab");
        }
        let mut idxs = Vec::new();
        for uid in uids {
            idxs.push(
                self.fragments
                    .iter()
                    .position(|f| f.uid == *uid)
                    .ok_or("a marked contig is gone")?,
            );
        }
        idxs.sort_unstable();
        self.begin_edit();
        let dest = idxs[0];
        let moving: Vec<Fragment> = idxs.iter().rev().map(|i| self.fragments.remove(*i)).collect();
        let moving: Vec<Fragment> = moving.into_iter().rev().collect();
        let ids: Vec<u64> = moving.iter().map(|f| f.uid).collect();
        self.fragments.splice(dest..dest, moving);
        self.edits.push(EditRecord {
            summary: format!("consolidated {} marked contigs", ids.len()),
        });
        self.bump();
        Ok(ids)
    }
}

fn default_tracks() -> Vec<Track> {
    vec![
        track("3p_telomere", TrackKind::Telomere3, [221, 204, 119, 255], false),
        track("5p_telomere", TrackKind::Telomere5, [119, 170, 221, 255], false),
        track("telomere", TrackKind::Telomere, [200, 160, 80, 255], false),
        track("coverage", TrackKind::Coverage, [80, 180, 120, 255], false),
        track("gap", TrackKind::Gap, [220, 80, 80, 255], false),
        track("repeat_density", TrackKind::Repeat, [160, 100, 200, 255], false),
        track("pixel_density", TrackKind::PixelDensity, [80, 160, 210, 255], true),
    ]
}

fn track(name: &str, kind: TrackKind, color: [u8; 4], on: bool) -> Track {
    Track {
        name: name.to_string(),
        on,
        color,
        height: 1.0,
        scale: 1.0,
        line: 1.5,
        label: true,
        kind,
    }
}

fn directed(left: &Fragment, right: &Fragment) -> f32 {
    let d = (left.right_ref() - right.left_ref()).abs() as f32;
    let base = (-d / 30.0).exp();
    if right.left_ref() == left.right_ref() + 1 {
        base + 0.08
    } else if left.right_ref() == right.left_ref() + 1 {
        base - 0.05
    } else {
        base
    }
}

fn link_sort(items: Vec<Fragment>, threshold: f32, min_pixels: u32, mode: SortMode) -> (Vec<Fragment>, usize) {
    let pass_threshold = match mode {
        SortMode::DeepFuse => threshold * 0.5,
        _ => threshold,
    };
    let mut chains: Vec<Vec<Fragment>> = items.into_iter().map(|frag| vec![frag]).collect();
    let mut pairs: Vec<(f32, u64, u64)> = Vec::new();
    for i in 0..chains.len() {
        for j in (i + 1)..chains.len() {
            let a = &chains[i][0];
            let b = &chains[j][0];
            if a.pixels < min_pixels || b.pixels < min_pixels {
                continue;
            }
            let score = directed(a, b).max(directed(b, a));
            if score >= pass_threshold {
                pairs.push((score, a.uid, b.uid));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut links = 0;
    for (_score, u, v) in pairs {
        if join_pair(&mut chains, u, v, pass_threshold) {
            links += 1;
        }
    }
    if mode == SortMode::Fuse {
        let relaxed = threshold * 0.75;
        loop {
            let mut joined = false;
            let ends: Vec<(u64, u64)> = chains
                .iter()
                .filter_map(|c| Some((c.first()?.uid, c.last()?.uid)))
                .collect();
            let mut best: Option<(f32, u64, u64)> = None;
            for i in 0..ends.len() {
                for j in (i + 1)..ends.len() {
                    for u in [ends[i].0, ends[i].1] {
                        for v in [ends[j].0, ends[j].1] {
                            if let (Some(fu), Some(fv)) = (find_frag(&chains, u), find_frag(&chains, v)) {
                                let score = directed(fu, fv).max(directed(fv, fu));
                                if score >= relaxed && best.map(|(s, _, _)| score > s).unwrap_or(true) {
                                    best = Some((score, u, v));
                                }
                            }
                        }
                    }
                }
            }
            if let Some((_, u, v)) = best {
                if join_pair(&mut chains, u, v, relaxed) {
                    links += 1;
                    joined = true;
                }
            }
            if !joined {
                break;
            }
        }
    }
    let mut out = Vec::new();
    for chain in chains {
        out.extend(chain);
    }
    (out, links)
}

fn find_frag<'a>(chains: &'a [Vec<Fragment>], uid: u64) -> Option<&'a Fragment> {
    chains.iter().flatten().find(|f| f.uid == uid)
}

fn join_pair(chains: &mut Vec<Vec<Fragment>>, u: u64, v: u64, threshold: f32) -> bool {
    let ia = chains.iter().position(|c| c.iter().any(|f| f.uid == u));
    let ib = chains.iter().position(|c| c.iter().any(|f| f.uid == v));
    let (Some(ia), Some(ib)) = (ia, ib) else {
        return false;
    };
    if ia == ib {
        return false;
    }
    let a_end = chains[ia].first().map(|f| f.uid) == Some(u) || chains[ia].last().map(|f| f.uid) == Some(u);
    let b_end = chains[ib].first().map(|f| f.uid) == Some(v) || chains[ib].last().map(|f| f.uid) == Some(v);
    if !a_end || !b_end {
        return false;
    }
    let Some((merged, score)) = concat_at_ends(&chains[ia], &chains[ib], u, v) else {
        return false;
    };
    if score < threshold {
        return false;
    }
    let (lo, hi) = if ia < ib { (ia, ib) } else { (ib, ia) };
    chains.remove(hi);
    chains.remove(lo);
    chains.insert(lo, merged);
    true
}

fn concat_at_ends(a: &[Fragment], b: &[Fragment], u: u64, v: u64) -> Option<(Vec<Fragment>, f32)> {
    let mut best: Option<(Vec<Fragment>, f32)> = None;
    for (left_id, right_id, left, right) in [(u, v, a, b), (v, u, b, a)] {
        let mut l = left.to_vec();
        let mut r = right.to_vec();
        if l.last().map(|f| f.uid) != Some(left_id) {
            if l.first().map(|f| f.uid) == Some(left_id) {
                reverse_chain(&mut l);
            } else {
                continue;
            }
        }
        if r.first().map(|f| f.uid) != Some(right_id) {
            if r.last().map(|f| f.uid) == Some(right_id) {
                reverse_chain(&mut r);
            } else {
                continue;
            }
        }
        let score = directed(l.last().unwrap(), r.first().unwrap());
        l.extend(r);
        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            best = Some((l, score));
        }
    }
    best
}

/// `sample.alg.pretext` becomes `sample.alg.agp`. A name the user typed is kept,
/// with `.agp` added when they left the extension off.
pub fn agp_export_name(map_name: &str) -> String {
    let base = map_name.rsplit(['/', '\\']).next().unwrap_or(map_name).trim();
    let stem = strip_pretext_suffix(base);
    let stem = if stem.is_empty() { "map" } else { stem };
    ensure_agp_suffix(stem)
}

pub fn ensure_agp_suffix(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "map.agp".to_string();
    }
    if name.to_ascii_lowercase().ends_with(".agp") {
        name.to_string()
    } else {
        format!("{name}.agp")
    }
}

fn strip_pretext_suffix(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix(".pretext") {
        name[..stripped.len()].trim_end_matches('.')
    } else {
        name
    }
}

fn reverse_chain(chain: &mut [Fragment]) {
    chain.reverse();
    for frag in chain {
        frag.inverted = !frag.inverted;
    }
}

pub fn contact_at(session: &Session, x: u32, y: u32, index: &[usize]) -> f32 {
    let Some(&ix) = index.get(x as usize) else {
        return 0.0;
    };
    let Some(&iy) = index.get(y as usize) else {
        return 0.0;
    };
    let fx = &session.fragments[ix];
    let fy = &session.fragments[iy];
    let mut acc = 0u32;
    let mut local_x = 0u32;
    let mut local_y = 0u32;
    for (i, frag) in session.fragments.iter().enumerate() {
        if i == ix {
            local_x = x - acc;
        }
        if i == iy {
            local_y = y - acc;
        }
        acc += frag.pixels;
    }
    let d = (fx.ref_at(local_x) - fy.ref_at(local_y)).unsigned_abs() as f32;
    (-d / 7.5).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_covers_the_map_and_is_shuffled() {
        let session = Session::demo();
        assert_eq!(session.total_pixels(), MAP_PIXELS);
        let sources: Vec<_> = session.fragments.iter().map(|f| f.source).collect();
        assert_ne!(sources, (0..sources.len()).collect::<Vec<_>>());
    }

    #[test]
    fn union_find_restores_reference_order() {
        let mut session = Session::demo();
        session.link_threshold = 0.2;
        session.sort_mode = SortMode::DeepFuse;
        let links = session.sort_fragments(None);
        assert!(links > 0, "expected some links");
        let sources: Vec<_> = session.fragments.iter().map(|f| f.source).collect();
        assert_eq!(sources, (0..sources.len()).collect::<Vec<_>>());
        assert_eq!(session.total_pixels(), MAP_PIXELS);
    }

    #[test]
    fn a_high_threshold_does_not_reorder() {
        let mut session = Session::demo();
        let before: Vec<_> = session.fragments.iter().map(|f| f.uid).collect();
        session.link_threshold = 2.0;
        session.sort_mode = SortMode::UnionFind;
        assert_eq!(session.sort_fragments(None), 0);
        let after: Vec<_> = session.fragments.iter().map(|f| f.uid).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn split_then_glue_round_trips() {
        let mut session = Session::demo();
        let before = session.fragments.len();
        assert!(session.split_at_pixel(10));
        assert_eq!(session.fragments.len(), before + 1);
        assert_eq!(session.total_pixels(), MAP_PIXELS);
        let glued = session.glue_adjacent(None);
        assert_eq!(glued, 1);
        assert_eq!(session.fragments.len(), before);
        assert_eq!(session.total_pixels(), MAP_PIXELS);
    }

    #[test]
    fn invert_mirrors_contacts_along_a_contig() {
        let mut session = Session::demo();
        let index = 2;
        let uid = session.fragments[index].uid;
        let (side, before) = session.raster_rgba(session.map_pixels);
        session.invert_uids(&[uid]).unwrap();
        assert!(session.fragments.iter().any(|frag| frag.uid == uid && frag.inverted));
        let (side_after, after) = session.raster_rgba(session.map_pixels);
        assert_eq!(side, side_after);
        let changed = before
            .chunks(4)
            .zip(after.chunks(4))
            .filter(|(left, right)| left != right)
            .count();
        assert!(
            changed > side as usize,
            "inverting one contig should repaint a strip of the map, changed {changed} pixels"
        );
    }

    #[test]
    fn place_moves_a_fragment_without_losing_pixels() {
        let mut session = Session::demo();
        let uid = session.fragments[0].uid;
        let source = session.fragments[0].source;
        session.place_uids(&[uid], MAP_PIXELS).unwrap();
        assert_eq!(session.fragments.last().unwrap().source, source);
        assert_eq!(session.total_pixels(), MAP_PIXELS);
        assert!(session.undo_edit());
        assert_eq!(session.fragments[0].uid, uid);
    }

    #[test]
    fn tagged_fragments_move_to_the_end() {
        let mut session = Session::demo();
        let n = session.move_tags_to_end(1 << 0);
        assert!(n >= 1);
        assert!(session.fragments.last().unwrap().meta & 1 != 0);
        assert_eq!(session.total_pixels(), MAP_PIXELS);
    }

    #[test]
    fn agp_round_trip_keeps_names() {
        let mut session = Session::demo();
        session.sort_fragments(None);
        let agp = session.to_agp();
        assert!(agp.contains("ptg000000l"));
        assert_eq!(session.agp_file_name(), "demo.agp");
        assert_eq!(
            super::agp_export_name("idOchAnnu3_RC-5104_normal.alg.pretext"),
            "idOchAnnu3_RC-5104_normal.alg.agp"
        );
        assert_eq!(super::agp_export_name(r"maps\Sample.PRETEXT"), "Sample.agp");
        assert_eq!(super::ensure_agp_suffix("my_assembly"), "my_assembly.agp");
        assert_eq!(super::ensure_agp_suffix("custom.agp"), "custom.agp");
        let mut shuffled = Session::demo();
        let n = shuffled.load_agp(&agp).unwrap();
        assert!(n > 0);
        assert_eq!(shuffled.total_pixels(), MAP_PIXELS);
    }

    #[test]
    fn pixel_cut_respects_the_threshold() {
        let mut gentle = Session::demo();
        gentle.cut_threshold = 0.5;
        assert_eq!(gentle.pixel_cut(None), 0);
        let mut session = Session::demo();
        session.cut_threshold = 0.05;
        let cuts = session.pixel_cut(None);
        assert!(cuts >= 1);
        assert_eq!(session.total_pixels(), MAP_PIXELS);
    }
}
