//! The feeling rating suggested at the entry of a drink (SPEC.md §5.3, §5.9).
//!
//! The suggestion reads the blood alcohol concentration the user sees, i.e.
//! rounded to two decimals, and applies the scale of §5.9 to it, each band
//! including its lower bound (user decision of 2026-10-25): 0.199 g/L is shown
//! as 0.20 and suggests 2, like 0.20 itself.
//!
//! | Rounded concentration (g/L) | Suggested rating |
//! |---|---|
//! | below 0.20 | 1 |
//! | from 0.20, below 0.50 | 2 |
//! | from 0.50, below 0.80 | 3 |
//! | from 0.80, below 1.20 | 4 |
//! | from 1.20, below 1.80 | 5 |
//! | from 1.80 | 6 |
//!
//! The suggestion never exceeds [`SUGGESTED_RATING_MAX`]: the levels above it
//! are the user's to pick, never proposed (SPEC.md §5.9).

/// Lowest rating ever suggested: no effect, nearly sober.
pub const SUGGESTED_RATING_MIN: u8 = 1;

/// Highest rating ever suggested, whatever the concentration (SPEC.md §5.9).
pub const SUGGESTED_RATING_MAX: u8 = 6;

/// Lower bound of each band above the first, in hundredths of a gram per
/// litre, with the rating it suggests, highest first.
const BANDS: [(i64, u8); 5] = [(180, 6), (120, 5), (80, 4), (50, 3), (20, 2)];

/// The feeling rating suggested for a blood alcohol concentration, in grams
/// per litre.
///
/// The concentration is first rounded to two decimals, half away from zero —
/// the value shown — then placed in the scale of the module, each band
/// including its lower bound. A negative concentration, which the engine never
/// gives, rounds into the first band, and so does `NaN`; an infinite one
/// suggests [`SUGGESTED_RATING_MAX`].
#[must_use]
pub fn suggested_rating(bac_g_per_l: f64) -> u8 {
    // The cast saturates, and takes `NaN` to zero: the bands are compared in
    // whole hundredths, where the bounds are exact.
    let hundredths = (bac_g_per_l * 100.0).round() as i64;
    BANDS
        .iter()
        .find(|(lower, _)| hundredths >= *lower)
        .map_or(SUGGESTED_RATING_MIN, |(_, rating)| *rating)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_band_includes_its_lower_bound() {
        for (bac, rating) in [(0.20, 2), (0.50, 3), (0.80, 4), (1.20, 5), (1.80, 6)] {
            assert_eq!(suggested_rating(bac), rating, "{bac} g/L");
        }
    }

    #[test]
    fn just_below_a_bound_once_rounded_stays_in_the_band_under_it() {
        for (bac, rating) in [
            (0.194_9, 1),
            (0.494_9, 2),
            (0.794_9, 3),
            (1.194_9, 4),
            (1.794_9, 5),
        ] {
            assert_eq!(suggested_rating(bac), rating, "{bac} g/L");
        }
    }

    #[test]
    fn the_scale_reads_the_value_rounded_to_two_decimals() {
        // User decision of 2026-10-25: 0.199 is shown as 0.20, and suggests
        // what 0.20 suggests. The criterion "0.199 → 1" of #73 is a typo.
        assert_eq!(suggested_rating(0.199), 2);
        assert_eq!(suggested_rating(0.195), 2);
        assert_eq!(suggested_rating(1.795), 6);
    }

    #[test]
    fn a_sober_body_is_suggested_the_lowest_rating() {
        assert_eq!(suggested_rating(0.0), SUGGESTED_RATING_MIN);
        assert_eq!(suggested_rating(0.1), 1);
    }

    #[test]
    fn the_suggestion_never_exceeds_six() {
        for bac in [3.0, 10.0, f64::MAX, f64::INFINITY] {
            assert_eq!(suggested_rating(bac), SUGGESTED_RATING_MAX, "{bac} g/L");
        }
        assert_eq!(SUGGESTED_RATING_MAX, 6);
    }

    #[test]
    fn a_value_the_engine_never_gives_falls_in_the_first_band() {
        for bac in [-0.5, -f64::INFINITY, f64::NAN] {
            assert_eq!(suggested_rating(bac), SUGGESTED_RATING_MIN, "{bac} g/L");
        }
    }

    #[test]
    fn every_rating_suggested_lies_within_the_suggested_scale() {
        let mut previous = SUGGESTED_RATING_MIN;
        for thousandths in 0..=5_000 {
            let rating = suggested_rating(f64::from(thousandths) / 1000.0);
            assert!(
                (SUGGESTED_RATING_MIN..=SUGGESTED_RATING_MAX).contains(&rating),
                "{thousandths} mg/L → {rating}"
            );
            assert!(rating >= previous, "the scale went down at {thousandths}");
            previous = rating;
        }
    }
}
