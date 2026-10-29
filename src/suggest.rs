//! Contact-map scan that proposes curation edits.
//!
//! The scan reads the contacts already on screen. Accepting a suggestion calls
//! the same split, invert, and move functions as the edit tools. A later scan
//! drops steps whose pixels moved.

use crate::model::Session;

const MAX_SUGGESTIONS: usize = 8;
const MOVE_MARGIN: f32 = 0.18;
const MOVE_MIN: f32 = 0.45;
const INVERT_MARGIN: f32 = 0.18;
const INVERT_MIN: f32 = 0.45;
const SPLIT_LOCAL_MIN: f32 = 0.30;
const SPLIT_CROSS_RATIO: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuggestId {
    Split { uid: u64, pixel: u32 },
    Invert { uid: u64 },
    Move { uid: u64, after: u64 },
}

#[derive(Clone, Debug)]
pub enum SuggestOp {
    Split { uid: u64, pixel: u32 },
    Invert { uid: u64 },
    Move { uid: u64, after: u64, dest: u32 },
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub id: SuggestId,
    pub score: f32,
    pub reason: String,
    pub op: SuggestOp,
}

pub struct ContactGrid {
    pub side: u32,
    pub values: Vec<f32>,
    pub owner: Vec<usize>,
    pub pixel: Vec<u32>,
}

impl ContactGrid {
    fn get(&self, x: u32, y: u32) -> f32 {
        if self.side == 0 || x >= self.side || y >= self.side {
            return 0.0;
        }
        self.values[(y * self.side + x) as usize]
    }
}

struct FragInfo {
    uid: u64,
    name: String,
    start_px: u32,
    end_px: u32,
}

struct Span {
    index: usize,
    start: u32,
    end: u32,
}

pub fn layout_stamp(session: &Session) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for frag in &session.fragments {
        hash ^= frag.uid;
        hash = hash.wrapping_mul(0x100000001b3);
        hash ^= frag.pixels as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        hash ^= u64::from(frag.inverted);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn suggest(session: &Session, skips: &[SuggestId]) -> Vec<Suggestion> {
    let grid = session.contact_grid();
    let pieces = frag_info(session);
    propose(&grid, &pieces, skips, session.cut_smallest)
}

pub fn apply(session: &mut Session, suggestion: &Suggestion) -> Result<(), String> {
    match &suggestion.op {
        SuggestOp::Split { uid, pixel } => {
            let inside = session
                .fragment_at(*pixel)
                .is_some_and(|index| session.fragments[index].uid == *uid);
            if !inside {
                return Err("That cut no longer sits inside the contig.".to_string());
            }
            if session.split_pixel(*pixel) {
                Ok(())
            } else {
                Err("That pixel is on a fragment edge.".to_string())
            }
        }
        SuggestOp::Invert { uid } => session.invert_uids(&[*uid]).map_err(str::to_string),
        SuggestOp::Move { uid, dest, .. } => session.place_uids(&[*uid], *dest).map_err(str::to_string),
    }
}

pub fn focus(session: &mut Session, suggestion: &Suggestion) {
    match &suggestion.op {
        SuggestOp::Split { pixel, .. } => {
            let pad = (session.map_pixels / 24).max(12);
            session.zoom_to_pixels(pixel.saturating_sub(pad), pixel.saturating_add(pad));
        }
        SuggestOp::Invert { uid } => {
            if let Some(index) = session.fragments.iter().position(|frag| frag.uid == *uid) {
                session.focus_fragment(index);
            }
        }
        SuggestOp::Move { uid, after, .. } => {
            let range = |id: u64| -> Option<(u32, u32)> {
                let mut acc = 0u32;
                for frag in &session.fragments {
                    let next = acc + frag.pixels;
                    if frag.uid == id {
                        return Some((acc, next));
                    }
                    acc = next;
                }
                None
            };
            let Some((a0, a1)) = range(*uid) else {
                return;
            };
            let Some((b0, b1)) = range(*after) else {
                session.zoom_to_pixels(a0, a1);
                return;
            };
            session.zoom_to_pixels(a0.min(b0), a1.max(b1));
        }
    }
}

fn frag_info(session: &Session) -> Vec<FragInfo> {
    let mut acc = 0u32;
    session
        .fragments
        .iter()
        .map(|frag| {
            let start_px = acc;
            acc += frag.pixels;
            FragInfo {
                uid: frag.uid,
                name: frag.display_name(),
                start_px,
                end_px: acc,
            }
        })
        .collect()
}

fn propose(grid: &ContactGrid, pieces: &[FragInfo], skips: &[SuggestId], min_cut: u32) -> Vec<Suggestion> {
    if grid.side < 2 || pieces.is_empty() {
        return Vec::new();
    }
    let spans = spans_of(grid);
    let mut out = Vec::new();
    out.extend(propose_splits(grid, pieces, &spans, min_cut));
    out.extend(propose_inverts(grid, pieces, &spans));
    out.extend(propose_moves(grid, pieces, &spans));
    out.retain(|suggestion| !skips.contains(&suggestion.id));
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(MAX_SUGGESTIONS);
    out
}

fn spans_of(grid: &ContactGrid) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut bin = 0u32;
    while bin < grid.side {
        let index = grid.owner[bin as usize];
        let start = bin;
        bin += 1;
        while bin < grid.side && grid.owner[bin as usize] == index {
            bin += 1;
        }
        spans.push(Span { index, start, end: bin });
    }
    spans
}

fn propose_splits(grid: &ContactGrid, pieces: &[FragInfo], spans: &[Span], min_cut: u32) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for span in spans {
        let Some(info) = pieces.get(span.index) else {
            continue;
        };
        let width = span.end.saturating_sub(span.start);
        if width < 8 {
            continue;
        }
        let window = 2u32;
        let lo = info.start_px.saturating_add(min_cut);
        let hi = info.end_px.saturating_sub(min_cut);
        if lo >= hi {
            continue;
        }
        let mut best: Option<(u32, f32)> = None;
        for cut in (span.start + window)..(span.end - window) {
            let left = (cut - window, cut);
            let right = (cut, cut + window);
            let cross = mean_block(grid, left.0, left.1, right.0, right.1);
            let local = 0.5
                * (mean_block(grid, left.0, left.1, left.0, left.1) + mean_block(grid, right.0, right.1, right.0, right.1));
            if local < SPLIT_LOCAL_MIN || cross > local * SPLIT_CROSS_RATIO {
                continue;
            }
            let raw = grid.pixel[cut as usize];
            let pixel = raw.clamp(lo, hi - 1);
            if pixel <= info.start_px || pixel >= info.end_px {
                continue;
            }
            let score = local - cross;
            if best.map(|(_, prev)| score > prev).unwrap_or(true) {
                best = Some((pixel, score));
            }
        }
        let Some((pixel, score)) = best else {
            continue;
        };
        out.push(Suggestion {
            id: SuggestId::Split { uid: info.uid, pixel },
            score,
            reason: format!(
                "Split {} at pixel {pixel}. Contact across this point is weaker than on either side.",
                info.name
            ),
            op: SuggestOp::Split { uid: info.uid, pixel },
        });
    }
    out
}

fn propose_inverts(grid: &ContactGrid, pieces: &[FragInfo], spans: &[Span]) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for (place, span) in spans.iter().enumerate() {
        let Some(info) = pieces.get(span.index) else {
            continue;
        };
        let mine = edge_width(span.end - span.start);
        if mine == 0 {
            continue;
        }
        let mut current = 0.0;
        let mut flipped = 0.0;
        let mut n = 0.0;
        if place > 0 {
            let left = &spans[place - 1];
            let left_w = edge_width(left.end - left.start);
            if left_w > 0 {
                let left_end = end_bins(left.start, left.end, false, left_w);
                let my_left = end_bins(span.start, span.end, true, mine);
                let my_right = end_bins(span.start, span.end, false, mine);
                current += mean_block(grid, left_end.0, left_end.1, my_left.0, my_left.1);
                flipped += mean_block(grid, left_end.0, left_end.1, my_right.0, my_right.1);
                n += 1.0;
            }
        }
        if place + 1 < spans.len() {
            let right = &spans[place + 1];
            let right_w = edge_width(right.end - right.start);
            if right_w > 0 {
                let right_start = end_bins(right.start, right.end, true, right_w);
                let my_left = end_bins(span.start, span.end, true, mine);
                let my_right = end_bins(span.start, span.end, false, mine);
                current += mean_block(grid, my_right.0, my_right.1, right_start.0, right_start.1);
                flipped += mean_block(grid, my_left.0, my_left.1, right_start.0, right_start.1);
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        current /= n;
        flipped /= n;
        if flipped < current + INVERT_MARGIN || flipped < INVERT_MIN {
            continue;
        }
        out.push(Suggestion {
            id: SuggestId::Invert { uid: info.uid },
            score: flipped - current,
            reason: format!(
                "Invert {}. The ends meet the neighbours more strongly the other way.",
                info.name
            ),
            op: SuggestOp::Invert { uid: info.uid },
        });
    }
    out
}

fn propose_moves(grid: &ContactGrid, pieces: &[FragInfo], spans: &[Span]) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let widths: Vec<u32> = spans.iter().map(|span| edge_width(span.end - span.start)).collect();
    for (place, span) in spans.iter().enumerate() {
        if widths[place] == 0 {
            continue;
        }
        let Some(anchor) = pieces.get(span.index) else {
            continue;
        };
        let anchor_end = end_bins(span.start, span.end, false, widths[place]);
        let current = if place + 1 < spans.len() && widths[place + 1] > 0 {
            let next = &spans[place + 1];
            let next_start = end_bins(next.start, next.end, true, widths[place + 1]);
            mean_block(grid, anchor_end.0, anchor_end.1, next_start.0, next_start.1)
        } else {
            0.0
        };
        let mut best: Option<(usize, f32)> = None;
        for (other_place, other) in spans.iter().enumerate() {
            if other_place == place || other_place == place + 1 || widths[other_place] == 0 {
                continue;
            }
            let other_start = end_bins(other.start, other.end, true, widths[other_place]);
            let score = mean_block(grid, anchor_end.0, anchor_end.1, other_start.0, other_start.1);
            if best.map(|(_, prev)| score > prev).unwrap_or(true) {
                best = Some((other_place, score));
            }
        }
        let Some((other_place, score)) = best else {
            continue;
        };
        if score < current + MOVE_MARGIN || score < MOVE_MIN {
            continue;
        }
        let Some(moving) = pieces.get(spans[other_place].index) else {
            continue;
        };
        out.push(Suggestion {
            id: SuggestId::Move { uid: moving.uid, after: anchor.uid },
            score: score - current,
            reason: format!(
                "Move {} to just after {}. Their contact is stronger than the current join.",
                moving.name, anchor.name
            ),
            op: SuggestOp::Move {
                uid: moving.uid,
                after: anchor.uid,
                dest: anchor.end_px,
            },
        });
    }
    out
}

fn edge_width(span: u32) -> u32 {
    if span >= 6 {
        3
    } else if span >= 2 {
        1
    } else {
        0
    }
}

/// `at_start` takes the low bins of the span. Otherwise the high bins.
fn end_bins(start: u32, end: u32, at_start: bool, width: u32) -> (u32, u32) {
    let width = width.min(end.saturating_sub(start)).max(1);
    if at_start {
        (start, start + width)
    } else {
        (end - width, end)
    }
}

fn mean_block(grid: &ContactGrid, x0: u32, x1: u32, y0: u32, y1: u32) -> f32 {
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut n = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += grid.get(x, y);
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Session;

    #[test]
    fn shuffled_demo_moves_join_reference_neighbours() {
        let session = Session::demo();
        let suggestions = suggest(&session, &[]);
        let mov = suggestions
            .iter()
            .find(|suggestion| matches!(suggestion.op, SuggestOp::Move { .. }))
            .expect("a shuffled map should propose a move");
        let SuggestOp::Move { uid, after, .. } = &mov.op else {
            unreachable!();
        };
        let after = session.fragments.iter().find(|frag| frag.uid == *after).unwrap();
        let moving = session.fragments.iter().find(|frag| frag.uid == *uid).unwrap();
        assert_eq!(after.ref_lo + after.pixels, moving.ref_lo);
        assert!(suggestions.iter().all(|suggestion| !matches!(suggestion.op, SuggestOp::Split { .. })));
    }

    #[test]
    fn an_inverted_fragment_between_its_neighbours_is_proposed() {
        let mut session = Session::demo();
        session.fragments.sort_by_key(|frag| frag.ref_lo);
        let uid = session.fragments[4].uid;
        session.invert_uids(&[uid]).unwrap();
        let suggestions = suggest(&session, &[]);
        assert!(
            suggestions.iter().any(|suggestion| matches!(suggestion.op, SuggestOp::Invert { uid: got } if got == uid)),
            "expected an invert of {uid}, got {:?}",
            suggestions.iter().map(|suggestion| suggestion.reason.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn accepting_a_move_drops_that_pair() {
        let mut session = Session::demo();
        let suggestions = suggest(&session, &[]);
        let mov = suggestions
            .iter()
            .find(|suggestion| matches!(suggestion.op, SuggestOp::Move { .. }))
            .unwrap()
            .clone();
        apply(&mut session, &mov).unwrap();
        let again = suggest(&session, &[]);
        assert!(again.iter().all(|suggestion| suggestion.id != mov.id));
    }

    #[test]
    fn a_break_inside_one_fragment_proposes_a_split() {
        let side = 20u32;
        let mut values = vec![0.05f32; (side * side) as usize];
        for y in 0..side {
            for x in 0..side {
                let same_half = (x < 10) == (y < 10);
                let near = x.abs_diff(y) <= 2;
                if same_half && near {
                    values[(y * side + x) as usize] = 0.8;
                }
            }
        }
        let grid = ContactGrid {
            side,
            values,
            owner: vec![0; side as usize],
            pixel: (0..side).collect(),
        };
        let pieces = vec![FragInfo {
            uid: 7,
            name: "ptg000007l".to_string(),
            start_px: 0,
            end_px: side,
        }];
        let suggestions = propose(&grid, &pieces, &[], 4);
        let split = suggestions.iter().find(|suggestion| matches!(suggestion.op, SuggestOp::Split { .. }));
        let split = split.expect("a weak crossing should propose a split");
        let SuggestOp::Split { pixel, .. } = split.op else {
            unreachable!();
        };
        assert!((8..12).contains(&pixel), "pixel {pixel}");
    }

    #[test]
    fn an_empty_map_suggests_nothing() {
        let session = Session::empty();
        assert!(suggest(&session, &[]).is_empty());
    }
}
