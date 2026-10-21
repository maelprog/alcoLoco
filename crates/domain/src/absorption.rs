//! Absorption profile — interchangeable by design (SPEC.md §6.4, §10.1).
//!
//! ⚠ [`Trapezoid`] is the DEFAULT model, a starting choice and not a product
//!   decision. To be settled before the V1 release — see issue #37.
//!
//! A profile says how the alcohol of one drink reaches the blood over time.
//! The engine of [`crate::curve`] only ever goes through the
//! [`AbsorptionProfile`] trait, so trying another model — pure Widmark, a
//! linear ramp, a first-order exponential absorption, the candidates #37 is to
//! confront on real curves — costs an implementation of that trait and nothing
//! else. A profile must declare a **finite** time after which the dose is
//! wholly absorbed, since a drinking episode only ends once nothing is left to
//! absorb: an asymptotic profile, such as the exponential one, declares a
//! cut-off — the time after which the unabsorbed remainder is deemed
//! negligible — and returns the whole dose from there on. Choosing that
//! cut-off is part of choosing the profile, in #37. Two are provided: [`Trapezoid`], the default, and [`PureWidmark`],
//! which ignores both durations, so that the same drinks can be compared under
//! both.
//!
//! The four tests of the crate marked
//! `#[ignore = "modèle d'absorption non arbitré — cf. #37"]` pin, for the
//! trapezoid and two variants, the closed form `κ × absorbed(τ) − β × τ` of a
//! single drink before its peak, computed by the test itself rather than by
//! [`crate::curve`]. Removing that attribute is the act that closes #37.

use crate::drink::Drink;
use crate::{EngineError, check_duration, trapezoid_absorbed};

/// Seconds in an hour, to turn the stored durations into the hours of §6.4.
const SECONDS_PER_HOUR: f64 = 3600.0;

/// The alcohol of one drink and the two durations that shape its absorption,
/// all validated.
///
/// `ingestion_hours` is the time taken to drink it, `absorption_hours` the time
/// each ingested fraction takes to reach the blood (SPEC.md §5.3, §6.4). What
/// these durations do to the curve is the profile's business.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dose {
    alcohol_grams: f64,
    ingestion_hours: f64,
    absorption_hours: f64,
}

impl Dose {
    /// A dose of `alcohol_grams` grams of pure alcohol, drunk over
    /// `ingestion_hours` and absorbed over `absorption_hours`.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidAlcoholMass`] when the mass is negative or not
    /// finite, [`EngineError::InvalidDuration`] when a duration is.
    pub fn new(
        alcohol_grams: f64,
        ingestion_hours: f64,
        absorption_hours: f64,
    ) -> Result<Self, EngineError> {
        if !(alcohol_grams.is_finite() && alcohol_grams >= 0.0) {
            return Err(EngineError::InvalidAlcoholMass {
                grams: alcohol_grams,
            });
        }
        Ok(Self {
            alcohol_grams,
            ingestion_hours: check_duration(ingestion_hours)?,
            absorption_hours: check_duration(absorption_hours)?,
        })
    }

    /// The dose of a validated drink: its pure alcohol, summed over its
    /// components (SPEC.md §6.1), and its two durations.
    ///
    /// [`Drink`] carries its volumes in millilitres. The database stores a
    /// quantity entered in cL as centilitres (migration `20261022090000`), and
    /// the API reads it back as such; the conversion to millilitres happens
    /// when the API builds the [`Drink`] from its own model (`ML_PER_CL` in
    /// `api::drink::validation`). Nothing here converts it again.
    ///
    /// # Errors
    ///
    /// None for a drink that passed its validation, whose mass and durations
    /// are finite and non-negative; the [`Result`] is that of [`Dose::new`].
    pub fn of_drink(drink: &Drink) -> Result<Self, EngineError> {
        Self::new(
            drink.pure_alcohol_grams(),
            drink.ingestion_duration_seconds() as f64 / SECONDS_PER_HOUR,
            drink.absorption_duration_seconds() as f64 / SECONDS_PER_HOUR,
        )
    }

    /// Mass of pure alcohol, in grams.
    #[must_use]
    pub fn alcohol_grams(&self) -> f64 {
        self.alcohol_grams
    }

    /// Ingestion duration, in hours.
    #[must_use]
    pub fn ingestion_hours(&self) -> f64 {
        self.ingestion_hours
    }

    /// Absorption duration, in hours.
    #[must_use]
    pub fn absorption_hours(&self) -> f64 {
        self.absorption_hours
    }
}

/// How the alcohol of one drink reaches the blood — the replaceable part of
/// the engine (SPEC.md §6.4, §10.1).
///
/// ⚠ The default implementation, [`Trapezoid`], is not a settled product
/// decision: the model is to be chosen before the V1 release, in issue #37.
///
/// An implementation describes one drink alone. The engine superposes drinks
/// by summing what each one has absorbed, applies the elimination once to the
/// body, groups the drinks into episodes and floors the displayed value at
/// zero (SPEC.md §6.3, §6.5): none of that belongs to a profile.
///
/// Contract, which the engine relies on:
///
/// * [`absorbed_grams`](Self::absorbed_grams) is `0` for a negative elapsed
///   time, never decreases, and never exceeds the mass of the dose;
/// * it equals that mass from
///   [`fully_absorbed_after_hours`](Self::fully_absorbed_after_hours) on, a
///   time which must be **finite** and non-negative — the engine refuses any
///   other with [`EngineError::InvalidDuration`]. A profile that only tends to
///   the whole dose declares a cut-off past which the remainder is deemed
///   negligible, and returns the whole dose from there (see the module
///   documentation; the cut-off belongs to #37);
/// * it is a pure function of its arguments, so that a curve can be replayed
///   bit for bit (SPEC.md §7).
pub trait AbsorptionProfile {
    /// Grams of the dose absorbed `elapsed_hours` after the first sip, i.e.
    /// `∫₀^τ R` for the rate of appearance `R` of the profile.
    ///
    /// The primitive is asked for rather than the rate, so that an
    /// instantaneous intake — a Dirac rate — is a plain step of the primitive.
    fn absorbed_grams(&self, dose: &Dose, elapsed_hours: f64) -> f64;

    /// Elapsed time, in hours, after which the whole dose is absorbed: finite
    /// and non-negative.
    fn fully_absorbed_after_hours(&self, dose: &Dose) -> f64;
}

/// The trapezoidal profile of SPEC.md §6.4 — the **default**, pending #37.
///
/// Linear ingestion over `a`, then uniform absorption of every ingested
/// fraction over `b`: the rate is the convolution of the two, a trapezoid,
/// computed by [`crate::absorption_rate_g_per_h`] and integrated in closed form
/// by [`crate::absorbed_alcohol_grams`]. It degenerates into a linear ramp
/// when one duration is zero and into pure Widmark when both are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Trapezoid;

impl AbsorptionProfile for Trapezoid {
    fn absorbed_grams(&self, dose: &Dose, elapsed_hours: f64) -> f64 {
        trapezoid_absorbed(
            dose.alcohol_grams,
            dose.ingestion_hours,
            dose.absorption_hours,
            elapsed_hours,
        )
    }

    fn fully_absorbed_after_hours(&self, dose: &Dose) -> f64 {
        dose.ingestion_hours + dose.absorption_hours
    }
}

/// Pure Widmark: the whole dose is in the blood at the first sip, whatever the
/// durations entered (SPEC.md §10.1, first row of the comparison table).
///
/// A comparison variant for #37, not the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PureWidmark;

impl AbsorptionProfile for PureWidmark {
    fn absorbed_grams(&self, dose: &Dose, elapsed_hours: f64) -> f64 {
        if elapsed_hours < 0.0 {
            0.0
        } else {
            dose.alcohol_grams
        }
    }

    fn fully_absorbed_after_hours(&self, _dose: &Dose) -> f64 {
        0.0
    }
}

/// The profile the application uses until #37 decides otherwise.
pub const DEFAULT_PROFILE: Trapezoid = Trapezoid;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drink::{ComponentDraft, DrinkDraft, QuantityUnit};

    const DERIVED: f64 = 1e-12;

    #[test]
    fn a_dose_refuses_a_negative_or_non_finite_input() {
        assert_eq!(
            Dose::new(-1.0, 0.0, 0.5),
            Err(EngineError::InvalidAlcoholMass { grams: -1.0 })
        );
        assert!(matches!(
            Dose::new(f64::NAN, 0.0, 0.5),
            Err(EngineError::InvalidAlcoholMass { .. })
        ));
        assert_eq!(
            Dose::new(10.0, -1.0, 0.5),
            Err(EngineError::InvalidDuration { hours: -1.0 })
        );
        assert_eq!(
            Dose::new(10.0, 0.0, f64::INFINITY),
            Err(EngineError::InvalidDuration {
                hours: f64::INFINITY
            })
        );
        // Zero is a mass (a soft drink) and a duration (pure Widmark).
        assert!(Dose::new(0.0, 0.0, 0.0).is_ok());
    }

    #[test]
    fn the_dose_of_a_drink_is_its_pure_alcohol_and_its_durations_in_hours() {
        // A 25 cL beer at 5 %, entered in cL: the drink holds millilitres.
        let drink = DrinkDraft {
            name: "Bière".to_owned(),
            unit: QuantityUnit::Cl,
            volume_total_ml: None,
            ingestion_duration_seconds: 1200,
            absorption_duration_seconds: 1800,
            components: vec![ComponentDraft {
                name: "Bière".to_owned(),
                abv_percent: 5.0,
                quantity: 250.0,
            }],
        }
        .validate()
        .expect("a valid drink");
        let dose = Dose::of_drink(&drink).expect("a valid dose");
        assert!(
            (dose.alcohol_grams() - 9.8625).abs() < DERIVED,
            "{}",
            dose.alcohol_grams()
        );
        assert!((dose.ingestion_hours() - 1.0 / 3.0).abs() < DERIVED);
        assert!((dose.absorption_hours() - 0.5).abs() < DERIVED);
    }

    #[test]
    fn the_trapezoid_profile_is_the_closed_form_primitive() {
        let dose = Dose::new(19.725, 1.0 / 3.0, 0.5).expect("valid");
        for tau in [-0.1, 0.0, 0.1, 0.4, 0.7, 5.0 / 6.0, 2.0] {
            let expected =
                crate::absorbed_alcohol_grams(19.725, 1.0 / 3.0, 0.5, tau).expect("valid");
            assert_eq!(
                Trapezoid.absorbed_grams(&dose, tau).to_bits(),
                expected.to_bits(),
                "τ = {tau}"
            );
        }
        assert!((Trapezoid.fully_absorbed_after_hours(&dose) - 5.0 / 6.0).abs() < DERIVED);
    }

    #[test]
    fn pure_widmark_ignores_the_durations() {
        let dose = Dose::new(19.725, 1.0 / 3.0, 0.5).expect("valid");
        assert_eq!(PureWidmark.absorbed_grams(&dose, -1e-9), 0.0);
        assert_eq!(PureWidmark.absorbed_grams(&dose, 0.0), 19.725);
        assert_eq!(PureWidmark.absorbed_grams(&dose, 0.2), 19.725);
        assert_eq!(PureWidmark.fully_absorbed_after_hours(&dose), 0.0);
    }
}
