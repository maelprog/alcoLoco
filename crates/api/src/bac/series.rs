//! From the stored drinks of a profile to the values of its curve.
//!
//! Each drink is computed with the settings version in force at its ingestion
//! time and the age then (SPEC.md §5.6, §10.0-K), resolved by the database for
//! each drink; this module turns them into the [`Intake`]s of the engine and
//! asks the engine for the values. The curve itself — superposition, episodes,
//! floor at zero — is `domain::curve`'s, and nothing of it is restated here.

use chrono::NaiveDate;
use domain::absorption::{Dose, Trapezoid};
use domain::curve::{self, Intake};
use domain::{BodyWater, ELIMINATION_RATE_G_PER_L_PER_H, EngineError};

use crate::drink::Drink;
use crate::error::ApiError;
use crate::profile::{ProfileSettings, Sex};
use crate::timestamp::Timestamp;

use super::query::{Grid, second_at_or_before};

/// Days in a mean Gregorian year, to express an age in years.
const DAYS_PER_YEAR: f64 = 365.2425;

/// A drink of the history, with the settings in force at its ingestion time.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDrink {
    /// The drink as stored.
    pub drink: Drink,
    /// The settings version in force at `drink.ingested_at`, `None` when the
    /// profile owns no version at all (SPEC.md §10.0-L, reserve (a) and (b)).
    pub settings: Option<ProfileSettings>,
}

/// The values of one profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Values {
    /// The value at the instant of the request.
    pub current: f64,
    /// One value per instant of the grid, in order.
    pub series: Vec<f64>,
}

/// Age, in years, at `at` of someone born on `birth_date`: whole days elapsed
/// over a mean Gregorian year. `None` before the birth date, where no age
/// exists.
#[must_use]
pub fn age_years(birth_date: NaiveDate, at: Timestamp) -> Option<f64> {
    let days = (at.date_naive() - birth_date).num_days();
    // Exact: a span a `timestamptz` can hold counts far fewer days than 2⁵³.
    let days = days as f64;
    (days >= 0.0).then(|| days / DAYS_PER_YEAR)
}

/// Total body water at `at` for the settings in force then: Watson with the
/// age at that instant, the Widmark fallback when no age exists yet (SPEC.md
/// §6.2).
///
/// # Errors
///
/// [`EngineError::NonPositiveBodyWater`] when the estimate is not strictly
/// positive.
pub fn body_water(settings: &ProfileSettings, at: Timestamp) -> Result<BodyWater, EngineError> {
    let sex = match settings.sex {
        Sex::Male => domain::Sex::Male,
        Sex::Female => domain::Sex::Female,
    };
    domain::total_body_water_litres(
        sex,
        settings.weight_kg,
        Some(settings.height_cm),
        age_years(settings.birth_date, at),
    )
}

/// The intake of one stored drink.
///
/// # Errors
///
/// A conflict when the settings in force give no positive body water — the
/// bounds of a profile accept such bodies on purpose (`domain::BodyWater`) —
/// and an internal failure when the drink is invalid or the profile owns no
/// settings version, neither of which the database lets happen through the
/// API.
pub fn intake(profile: uuid::Uuid, stored: &StoredDrink) -> Result<Intake, ApiError> {
    let drink = &stored.drink;
    let Some(settings) = &stored.settings else {
        return Err(ApiError::internal(format!(
            "profile {profile} owns no settings version for drink {}",
            drink.id
        )));
    };
    let valid = drink.to_domain().map_err(|broken| {
        ApiError::internal(format!("stored drink {} breaks {broken:?}", drink.id))
    })?;
    let dose = Dose::of_drink(&valid)
        .map_err(|error| ApiError::internal(format!("drink {}: {error}", drink.id)))?;
    let body_water = body_water(settings, drink.ingested_at).map_err(|error| {
        ApiError::conflict(format!(
            "the settings of profile {profile} in force at {} give no positive body water \
             ({error}): its curve cannot be computed",
            drink.ingested_at
        ))
    })?;
    Ok(Intake {
        started_at_seconds: second_at_or_before(drink.ingested_at),
        dose,
        body_water,
    })
}

/// The current value at `now` and the series on `grid`, from the intakes of
/// one profile.
///
/// # Errors
///
/// An internal failure when the engine refuses its input, which the intakes
/// built by [`intake`] never make it do.
pub fn values(intakes: &[Intake], grid: Grid, now: Timestamp) -> Result<Values, ApiError> {
    let engine = |error: EngineError| ApiError::internal(format!("curve refused: {error}"));
    let current = curve::bac_at(
        intakes,
        &Trapezoid,
        ELIMINATION_RATE_G_PER_L_PER_H,
        second_at_or_before(now),
    )
    .map_err(engine)?;
    let series = if grid.is_empty() {
        Vec::new()
    } else {
        curve::bac_series(
            intakes,
            &Trapezoid,
            ELIMINATION_RATE_G_PER_L_PER_H,
            grid.from,
            grid.to,
            grid.step,
        )
        .map_err(engine)?
        .into_iter()
        .map(|sample| sample.bac_g_per_l)
        .collect()
    };
    Ok(Values { current, series })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drink::Component;
    use crate::profile::QuantityUnit;
    use domain::absorption::PureWidmark;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a real instant")
    }

    fn born(text: &str) -> NaiveDate {
        text.parse().expect("a real date")
    }

    /// The reference body of SPEC.md §10.1: a man of 80 kg and 180 cm, 30
    /// years old on 2026-08-20.
    fn reference_settings() -> ProfileSettings {
        ProfileSettings {
            valid_from: at("2026-01-01T00:00:00Z"),
            weight_kg: 80.0,
            height_cm: 180.0,
            sex: Sex::Male,
            birth_date: born("1996-08-20"),
        }
    }

    /// The reference drink of SPEC.md §10.1: 50 cL at 5 %, drunk over 20 min
    /// and absorbed over 30 min.
    fn reference_drink(ingested_at: &str) -> Drink {
        Drink {
            id: uuid::Uuid::nil(),
            profile_id: uuid::Uuid::nil(),
            event_id: None,
            name: "Bière".to_owned(),
            ingested_at: at(ingested_at),
            ingestion_duration_seconds: 1200,
            absorption_duration_seconds: 1800,
            quantity_unit: QuantityUnit::Cl,
            volume_total_ml: None,
            feeling_rating: None,
            components: vec![Component {
                name: "Bière".to_owned(),
                abv_percent: 5.0,
                quantity: 50.0,
            }],
            created_at: at(ingested_at),
            updated_at: at(ingested_at),
        }
    }

    fn stored(ingested_at: &str, settings: ProfileSettings) -> StoredDrink {
        StoredDrink {
            drink: reference_drink(ingested_at),
            settings: Some(settings),
        }
    }

    fn grid(from: &str, to: &str, step: i64) -> Grid {
        Grid {
            from: at(from).timestamp(),
            to: at(to).timestamp(),
            step,
        }
    }

    #[test]
    fn the_age_is_counted_at_the_instant_in_mean_years() {
        let birth = born("1996-08-20");
        let days = 10_957.0; // 1996-08-20 → 2026-08-20
        assert_eq!(
            age_years(birth, at("2026-08-20T21:00:00Z")),
            Some(days / DAYS_PER_YEAR)
        );
        assert_eq!(age_years(birth, at("1996-08-20T00:00:00Z")), Some(0.0));
        assert_eq!(age_years(birth, at("1996-08-19T23:59:59Z")), None);
    }

    #[test]
    fn the_body_water_is_watson_with_the_age_at_ingestion() {
        let settings = reference_settings();
        let when = at("2026-08-20T21:00:00Z");
        let expected = domain::watson_total_body_water_litres(
            domain::Sex::Male,
            80.0,
            180.0,
            age_years(settings.birth_date, when).expect("born"),
        )
        .expect("positive");
        assert_eq!(body_water(&settings, when), Ok(expected));
        // Ten years later, the same settings give less water to a man.
        let later = body_water(&settings, at("2036-08-20T21:00:00Z")).expect("positive");
        assert!(later.litres() < expected.litres());
    }

    #[test]
    fn before_the_birth_date_the_body_water_falls_back_on_widmark() {
        let settings = reference_settings();
        assert_eq!(
            body_water(&settings, at("1990-01-01T00:00:00Z")),
            domain::widmark_total_body_water_litres(domain::Sex::Male, 80.0)
        );
    }

    #[test]
    fn a_body_too_small_for_watson_is_refused() {
        let tiny = ProfileSettings {
            weight_kg: 0.5,
            height_cm: 1.0,
            sex: Sex::Female,
            ..reference_settings()
        };
        assert!(matches!(
            body_water(&tiny, at("2026-08-20T21:00:00Z")),
            Err(EngineError::NonPositiveBodyWater { .. })
        ));
        let refused = intake(
            uuid::Uuid::nil(),
            &StoredDrink {
                settings: Some(tiny),
                ..stored("2026-08-20T21:00:00Z", reference_settings())
            },
        );
        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");
    }

    #[test]
    fn an_intake_carries_the_alcohol_in_millilitres_and_the_body_of_its_time() {
        let settings = reference_settings();
        let made = intake(
            uuid::Uuid::nil(),
            &stored("2026-08-20T21:00:00.900Z", settings.clone()),
        )
        .expect("an intake");
        // 50 cL at 5 %: 500 mL × 5 / 100 × 0.789.
        assert_eq!(
            made.dose.alcohol_grams(),
            domain::ingested_alcohol_grams(500.0, 5.0)
        );
        assert_eq!(made.dose.ingestion_hours(), 1200.0 / 3600.0);
        assert_eq!(made.dose.absorption_hours(), 1800.0 / 3600.0);
        // The start is the second of the ingestion, rounded down.
        assert_eq!(
            made.started_at_seconds,
            at("2026-08-20T21:00:00Z").timestamp()
        );
        assert_eq!(
            Ok(made.body_water),
            body_water(&settings, at("2026-08-20T21:00:00.900Z"))
        );
    }

    #[test]
    fn each_drink_takes_the_settings_of_its_own_time() {
        let heavier = ProfileSettings {
            weight_kg: 100.0,
            ..reference_settings()
        };
        let first = intake(
            uuid::Uuid::nil(),
            &stored("2026-08-20T21:00:00Z", reference_settings()),
        )
        .expect("an intake");
        let second =
            intake(uuid::Uuid::nil(), &stored("2026-08-20T21:00:00Z", heavier)).expect("an intake");
        assert!(second.body_water.litres() > first.body_water.litres());
    }

    #[test]
    fn a_drink_without_settings_is_an_internal_failure() {
        let orphan = StoredDrink {
            settings: None,
            ..stored("2026-08-20T21:00:00Z", reference_settings())
        };
        assert!(matches!(
            intake(uuid::Uuid::nil(), &orphan),
            Err(ApiError::Internal(_))
        ));
    }

    #[test]
    fn the_values_are_those_of_the_engine_on_the_grid_and_now() {
        let intakes = [
            intake(
                uuid::Uuid::nil(),
                &stored("2026-08-20T21:00:00Z", reference_settings()),
            ),
            intake(
                uuid::Uuid::nil(),
                &stored("2026-08-20T21:30:00Z", reference_settings()),
            ),
        ]
        .map(|made| made.expect("an intake"));
        let window = grid("2026-08-20T20:00:00Z", "2026-08-21T03:00:00Z", 300);
        let now = at("2026-08-20T21:47:30.5Z");
        let computed = values(&intakes, window, now).expect("values");

        let engine = curve::bac_series(
            &intakes,
            &Trapezoid,
            ELIMINATION_RATE_G_PER_L_PER_H,
            window.from,
            window.to,
            window.step,
        )
        .expect("a series");
        assert_eq!(
            computed.series,
            engine
                .iter()
                .map(|sample| sample.bac_g_per_l)
                .collect::<Vec<_>>()
        );
        assert_eq!(computed.series.len() as i64, window.len());
        assert_eq!(
            computed.current,
            curve::bac_at(
                &intakes,
                &Trapezoid,
                ELIMINATION_RATE_G_PER_L_PER_H,
                at("2026-08-20T21:47:30Z").timestamp(),
            )
            .expect("a value")
        );
        assert!(computed.current > 0.0);
        // Before the first drink and long after the last, nothing.
        assert_eq!(computed.series.first(), Some(&0.0));
        assert_eq!(computed.series.last(), Some(&0.0));
    }

    #[test]
    fn the_reference_drink_peaks_where_spec_10_1_says() {
        // SPEC.md §10.1: trapezoid peak 0.227 g/L at 45.7 min. The sample at
        // 46 min, a minute grid's nearest, rounds to the same 0.227.
        let intakes = [intake(
            uuid::Uuid::nil(),
            &stored("2026-08-20T21:00:00Z", reference_settings()),
        )
        .expect("an intake")];
        let window = grid("2026-08-20T21:00:00Z", "2026-08-20T23:00:00Z", 60);
        let series = values(&intakes, window, at("2026-08-20T21:00:00Z"))
            .expect("values")
            .series;
        let (peak_minute, peak) = series
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .expect("samples");
        assert_eq!(peak_minute, 46);
        assert!((peak - 0.227).abs() < 5e-4, "peak {peak}");
        // And C₀ = 0.347 g/L under pure Widmark, through the same intake.
        let c0 = curve::bac_at(
            &intakes,
            &PureWidmark,
            ELIMINATION_RATE_G_PER_L_PER_H,
            window.from,
        )
        .expect("a value");
        assert!((c0 - 0.347).abs() < 5e-4, "C0 {c0}");
    }

    #[test]
    fn an_empty_grid_gives_an_empty_series_and_still_a_current_value() {
        let intakes = [intake(
            uuid::Uuid::nil(),
            &stored("2026-08-20T21:00:00Z", reference_settings()),
        )
        .expect("an intake")];
        let empty = Grid {
            from: 10,
            to: 0,
            step: 60,
        };
        let computed = values(&intakes, empty, at("2026-08-20T21:30:00Z")).expect("values");
        assert!(computed.series.is_empty());
        assert!(computed.current > 0.0);
    }

    #[test]
    fn no_drink_is_zero_everywhere() {
        let window = grid("2026-08-20T20:00:00Z", "2026-08-20T21:00:00Z", 600);
        let computed = values(&[], window, at("2026-08-20T21:00:00Z")).expect("values");
        assert_eq!(computed.current, 0.0);
        assert_eq!(computed.series, vec![0.0; 7]);
    }
}
