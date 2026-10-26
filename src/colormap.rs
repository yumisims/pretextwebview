//! Colour ramps used by the contact-map view.
//! Names match PretextView's menu. The ramps are compact stand-ins for the
//! original 256-entry tables, so each menu entry is visually distinct.

pub const NAMES: &[&str] = &[
    "light_sunset (colorblind)",
    "viridis (colorblind)",
    "cividis (colorblind)",
    "Inferno",
    "Black Body",
    "Kindlmann",
    "Extended Kindlmann",
    "Three Wave Yellow-Grey-Blue",
    "Three Wave Blue-Green-Yellow",
    "Four Wave Grey-Red-Green",
    "Five Wave Yellow-Brown-Blue",
    "Blue 1",
    "Blue 2",
    "Blue 3",
    "Blue 4",
    "Blue-Orange Divergent",
    "Brown 1",
    "Brown 2",
    "Brown 3",
    "Green 1",
    "Green 2",
    "Green 3",
    "Green 4",
    "Mellow Rainbow",
    "Orange 1",
    "Orange 2",
    "Orange-Green-Blue-Gray",
    "Purple 1",
    "Purple 2",
    "Red 1",
    "Red 2",
    "Red 3",
    "Yellow 1",
    "Yellow 2",
    "Copy_texture(always at end)",
    "Fall",
];

/// Yellow–orange–red ramp. The low end is pale, the high end is dark red.
pub const FALL: usize = 35;

pub fn sample(index: usize, t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let [r, g, b] = match index {
        0 => stops(t, &[[255, 255, 212], [254, 217, 142], [254, 153, 41], [217, 95, 14], [153, 52, 4]]),
        1 => viridis(t),
        2 => stops(t, &[[0, 34, 78], [66, 90, 112], [144, 160, 120], [230, 230, 180], [255, 255, 200]]),
        3 => stops(t, &[[0, 0, 4], [40, 11, 84], [120, 28, 109], [220, 74, 30], [252, 255, 164]]),
        4 => stops(t, &[[0, 0, 0], [120, 0, 0], [230, 80, 0], [255, 200, 40], [255, 255, 220]]),
        5 => stops(t, &[[0, 0, 0], [40, 0, 80], [0, 90, 140], [40, 180, 80], [250, 240, 80]]),
        7 => stops(t, &[[250, 230, 40], [160, 160, 160], [40, 80, 180]]),
        8 => stops(t, &[[30, 70, 180], [40, 170, 90], [240, 220, 40]]),
        9 => stops(t, &[[80, 80, 80], [190, 40, 40], [40, 150, 70]]),
        10 => stops(t, &[[240, 220, 40], [140, 90, 40], [40, 70, 160], [20, 20, 60]]),
        15 => divergent(t, [30, 90, 180], [245, 245, 245], [210, 100, 30]),
        23 => stops(t, &[[80, 0, 120], [0, 90, 200], [0, 180, 80], [240, 220, 40], [200, 30, 30]]),
        26 => stops(t, &[[230, 120, 20], [40, 150, 70], [40, 90, 180], [180, 180, 180]]),
        34 => {
            let g = (t * 255.0) as u8;
            [g, g, g]
        }
        FALL => fall(t),
        n => named_ramp(n, t),
    };
    [r, g, b, 255]
}

fn named_ramp(index: usize, t: f32) -> [u8; 3] {
    let name = NAMES.get(index).copied().unwrap_or("");
    let (lo, hi) = if name.contains("Blue") {
        ([4, 12, 40], [170, 210, 255])
    } else if name.contains("Brown") {
        ([40, 20, 8], [230, 190, 130])
    } else if name.contains("Green") {
        ([4, 30, 12], [170, 240, 140])
    } else if name.contains("Orange") {
        ([50, 16, 0], [255, 200, 80])
    } else if name.contains("Purple") {
        ([24, 0, 40], [220, 170, 255])
    } else if name.contains("Red") {
        ([40, 0, 0], [255, 160, 140])
    } else if name.contains("Yellow") {
        ([40, 30, 0], [255, 245, 160])
    } else {
        ([0, 0, 0], [255, 255, 255])
    };
    // Shift the midpoint so Blue 1..4 are not identical.
    let bias = (index % 5) as f32 * 0.08;
    let u = (t * (1.0 + bias)).clamp(0.0, 1.0);
    lerp(lo, hi, u * u)
}

/// Pale at the bottom so a count of one is visible, dark red at the top.
fn fall(t: f32) -> [u8; 3] {
    let knee = 0.04;
    if t <= knee {
        return lerp([255, 248, 230], [255, 176, 56], t / knee);
    }
    stops(
        (t - knee) / (1.0 - knee),
        &[[255, 176, 56], [230, 96, 18], [170, 28, 8], [48, 6, 4]],
    )
}

fn viridis(t: f32) -> [u8; 3] {
    stops(
        t,
        &[[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]],
    )
}

fn divergent(t: f32, lo: [u8; 3], mid: [u8; 3], hi: [u8; 3]) -> [u8; 3] {
    if t < 0.5 {
        lerp(lo, mid, t * 2.0)
    } else {
        lerp(mid, hi, (t - 0.5) * 2.0)
    }
}

fn stops(t: f32, colors: &[[u8; 3]]) -> [u8; 3] {
    if colors.len() == 1 {
        return colors[0];
    }
    let scaled = t * (colors.len() - 1) as f32;
    let i = (scaled.floor() as usize).min(colors.len() - 2);
    let f = scaled - i as f32;
    lerp(colors[i], colors[i + 1], f)
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let c = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    [c(0), c(1), c(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fall_is_pale_at_the_bottom_and_dark_at_the_top() {
        let low = sample(FALL, 0.0);
        let floor = sample(FALL, 0.04);
        let high = sample(FALL, 1.0);
        let lum = |px: [u8; 4]| px[0] as u16 + px[1] as u16 + px[2] as u16;
        assert_eq!(NAMES[FALL], "Fall");
        assert!(low[0] > 240 && low[1] > 230, "low end {low:?}");
        assert!(floor[0] > 240 && floor[1] > 140 && floor[2] < 80, "floor {floor:?}");
        assert!(lum(low) > lum(high));
        assert!(high[0] < 80 && high[1] < 30, "high end {high:?}");
    }
}

/// PretextView gamma: three control points. `mid` is the value that maps to 0.5.
pub fn tone(v: f32, cp: [f32; 3]) -> f32 {
    let min = cp[0].clamp(0.0, 1.0);
    let max = cp[2].clamp(min, 1.0);
    let mid = cp[1].clamp(min, max);
    if (max - min) < 1e-4 {
        return if v >= max { 1.0 } else { 0.0 };
    }
    let x = ((v - min) / (max - min)).clamp(0.0, 1.0);
    let pivot = ((mid - min) / (max - min)).clamp(0.02, 0.98);
    let gamma = (0.5f32.ln() / pivot.ln()).clamp(0.12, 8.0);
    x.powf(gamma)
}
