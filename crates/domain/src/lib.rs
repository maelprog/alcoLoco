//! Blood alcohol computation engine.
//!
//! This crate is deliberately free of any I/O, database or web framework
//! dependency: it holds pure, deterministic functions so that it can be unit
//! tested in isolation and, later, extracted as a standalone service
//! (see SPEC.md §7 and §9).
//!
//! Only the reference figures that SPEC.md §6 states without ambiguity live
//! here for now. The engine itself is built by a later issue.

/// Density of ethanol, in grams per millilitre (SPEC.md §6.1).
pub const ETHANOL_DENSITY_G_PER_ML: f64 = 0.789;

/// Mass of pure alcohol, in grams, contained in a given volume of a beverage.
///
/// `volume_ml` is the volume of the beverage in millilitres, `abv_percent` its
/// alcohol by volume in percent. Reference formula of SPEC.md §6.1:
/// `A (g) = volume(mL) × degree(%) / 100 × 0.789`.
#[must_use]
pub fn ingested_alcohol_grams(volume_ml: f64, abv_percent: f64) -> f64 {
    volume_ml * abv_percent / 100.0 * ETHANOL_DENSITY_G_PER_ML
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_25_cl_beer_at_5_percent_holds_about_10_grams_of_alcohol() {
        let grams = ingested_alcohol_grams(250.0, 5.0);
        assert!(
            (grams - 9.8625).abs() < 1e-9,
            "unexpected alcohol mass: {grams}"
        );
    }

    #[test]
    fn a_soft_drink_holds_no_alcohol() {
        assert_eq!(ingested_alcohol_grams(330.0, 0.0), 0.0);
    }
}
