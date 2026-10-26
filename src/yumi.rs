//! Display curves for a pretext texture.
//!
//! A `.pretext` file stores each contact as a byte. Ed keeps PretextView's
//! power curve (`colormap::tone`). Yumi maps the whole range with the HiGlass
//! log, fixed for the whole map. Zero stays zero.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub enum SignalMethod {
    #[default]
    Pretext,
    Yumi,
}

impl<'de> serde::Deserialize<'de> for SignalMethod {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <String as serde::Deserialize>::deserialize(deserializer)?;
        match name.as_str() {
            "Yumi" => Ok(Self::Yumi),
            "Pretext" | "Lift" => Ok(Self::Pretext),
            other => Err(serde::de::Error::unknown_variant(other, &["Pretext", "Yumi"])),
        }
    }
}

/// Log-scale `value` inside the Min..Max window.
///
/// `limits` is the same `[min, mid, max]` triple as Pretext gamma. Mid is
/// unused: Yumi has no power pivot. A stored zero, and anything at or below
/// Min, stays black. Anything at or above Max saturates.
pub fn tone(value: f32, limits: [f32; 3]) -> f32 {
    let min = limits[0].clamp(0.0, 1.0);
    let max = limits[2].clamp(min, 1.0);
    let value = value.clamp(0.0, 1.0);
    if max - min < 1e-4 {
        return if value >= max { 1.0 } else { 0.0 };
    }
    if value <= min {
        return 0.0;
    }
    if value >= max {
        return 1.0;
    }
    let low = min * 255.0;
    let high = max * 255.0;
    let span = (high - low).max(1.0);
    let above = (value * 255.0 - low).max(0.0);
    ((above + 1.0).ln() / (span + 1.0).ln()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_stays_black_and_the_top_byte_saturates() {
        assert_eq!(tone(0.0, [0.0, 0.5, 1.0]), 0.0);
        assert_eq!(tone(1.0, [0.0, 0.5, 1.0]), 1.0);
    }

    #[test]
    fn a_weak_byte_takes_more_of_the_range_than_a_linear_map() {
        let weak = tone(16.0 / 255.0, [0.0, 0.5, 1.0]);
        assert!(weak > 0.45, "byte 16 mapped to {weak}");
        assert!(weak > 16.0 / 255.0 * 4.0);
    }

    #[test]
    fn the_curve_is_monotone_and_respects_the_window() {
        let limits = [0.1, 0.5, 0.8];
        let mut previous = 0.0;
        for step in 0..=255 {
            let value = tone(step as f32 / 255.0, limits);
            assert!(value + 1e-5 >= previous);
            previous = value;
        }
        assert_eq!(tone(0.05, limits), 0.0);
        assert_eq!(tone(0.9, limits), 1.0);
    }

    #[test]
    fn a_saved_lift_choice_loads_as_ed() {
        assert_eq!(
            serde_json::from_str::<SignalMethod>("\"Lift\"").unwrap(),
            SignalMethod::Pretext
        );
        assert_eq!(
            serde_json::from_str::<SignalMethod>("\"Yumi\"").unwrap(),
            SignalMethod::Yumi
        );
    }
}
