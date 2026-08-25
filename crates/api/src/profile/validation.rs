//! Bounds a written profile has to satisfy, and the reading of a request.
//!
//! Four of the five bounds below are **the ones the schema enforces**, restated
//! here so that a bad value comes back as a 400 naming the field rather than as
//! a 500 carrying a constraint name. Restating them is only safe while the two
//! agree, so the schema is asked directly rather than taken on trust: the
//! database-backed test `the_schema_enforces_exactly_the_bounds_the_api_restates`
//! writes the value *at* each bound and the value one `f64` inside it, and
//! requires the first refused and the second accepted. That pins the two numbers
//! together in both directions, on the live catalogue. A second, cheaper test
//! reads the migration SQL; it needs no database, and it can only say a clause is
//! *written* somewhere — never which clause is in force.
//!
//! The fifth, [`MAX_AGE_YEARS`], has no counterpart in the schema.
//!
//! # What this module does not do: the Watson degeneracy
//!
//! Read this before adding a bound here to "protect" the computation, and read
//! it if you are working on #16 — it is written for you.
//!
//! The male equation of SPEC.md §6.2 subtracts `0.09516 × age`, so for a small
//! enough body it reaches a **non-positive total body water** at a finite age.
//! `initial_bac_g_per_l` then divides by that value and hands back a negative
//! concentration. `crates/domain` puts **no guard on the sign**, so the failure
//! is neither impossible nor visible: it propagates as a plausible-looking
//! number.
//!
//! **The bounds below do not close it, and are not meant to.** Weight and height
//! are bounded away from zero and nothing more, which is a **product decision,
//! taken by the user on 2026-08-23**: the application is to accept as many real
//! bodies as it can — someone with dwarfism among them — so no anthropometric
//! floor is imposed. A floor tuned until a formula stops misbehaving would be a
//! bound describing an equation rather than a person, and that is exactly what
//! was refused.
//!
//! Measured against `crates/domain` itself, at the worst corner these bounds
//! accept, that is with weight and height approaching zero from above:
//!
//! ```text
//! crossing of TBW <= 0        25.715 years
//! MAX_AGE_YEARS                  130 years
//! ```
//!
//! The crossing is therefore **104 years inside** the oldest birth date this
//! module accepts, and a profile past it is reachable through the ordinary API.
//! Measured end to end on the request that reaches it: a body of 0.5 kg and 1 cm
//! at 30.6 years yields a total body water of −0.1894 L, and a single 25 cL beer
//! at 5 % then yields −41.97 g/L.
//!
//! The guard that closes this is `TBW > 0` inside `crates/domain`, which is
//! **issue #16**, recorded as a debt there. Nothing in this crate substitutes
//! for it, and no wording here should suggest otherwise.
//!
//! Every message is built with `format!` from the constant it quotes. A message
//! spelling a number out would drift the moment the bound moved, and no test
//! reads prose.

use chrono::NaiveDate;

use crate::error::{ApiError, FieldError};
use crate::timestamp::Timestamp;

use super::model::{ProfileRequest, ProfileSettings, QuantityUnit, SettingsRequest, Sex};

/// Exclusive lower bound on `weight_kg`, as the migration's `CHECK` spells it.
///
/// Positivity, and deliberately nothing more: no anthropometric floor is imposed,
/// so that no real body is turned away (user decision, 2026-08-23). It is not a
/// bound that makes the Watson equation of SPEC.md §6.2 well behaved — see the
/// module documentation.
pub const WEIGHT_KG_MIN_EXCLUSIVE: f64 = 0.0;

/// Exclusive upper bound on `weight_kg`, as the migration's `CHECK` spells it.
pub const WEIGHT_KG_MAX_EXCLUSIVE: f64 = 1000.0;

/// Exclusive lower bound on `height_cm`, as the migration's `CHECK` spells it.
///
/// Positivity only. Same reason and same caveat as [`WEIGHT_KG_MIN_EXCLUSIVE`].
pub const HEIGHT_CM_MIN_EXCLUSIVE: f64 = 0.0;

/// Exclusive upper bound on `height_cm`, as the migration's `CHECK` spells it.
pub const HEIGHT_CM_MAX_EXCLUSIVE: f64 = 300.0;

/// Inclusive lower bound on the default ingestion duration, in seconds.
///
/// Zero is allowed: an instantaneous ingestion is a supported case of the
/// absorption profile of SPEC.md §6.4, not a degenerate one.
pub const INGESTION_DURATION_MIN_SECONDS: i32 = 0;

/// Exclusive lower bound on the default absorption duration, in seconds.
///
/// Zero is refused here as it is in the schema: a null absorption duration
/// degenerates the trapezoid of SPEC.md §6.4.
pub const ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS: i32 = 0;

/// Largest age a birth date may imply, in whole years.
///
/// A bound on human longevity, and nothing else: it sits above every verified
/// lifespan — the oldest, Jeanne Calment, reached 122 — so it refuses only dates
/// no person could carry.
///
/// It is **not** what keeps the Watson equation of SPEC.md §6.2 out of trouble.
/// An earlier version of this comment claimed it was, and that was false: the
/// equation crosses zero at 25.715 years at the worst body this module accepts,
/// 104 years inside this bound. The module documentation carries the
/// measurement and names the issue that closes it.
pub const MAX_AGE_YEARS: u32 = 130;

/// A write that satisfied every bound.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidProfile {
    /// Name shown in the profile picker, trimmed.
    pub display_name: String,
    /// Unit the entry form pre-selects for a component.
    pub default_quantity_unit: QuantityUnit,
    /// Ingestion duration the entry form pre-fills, in seconds.
    pub default_ingestion_duration_seconds: i32,
    /// Absorption duration the entry form pre-fills, in seconds.
    pub default_absorption_duration_seconds: i32,
    /// The four physiological parameters and the instant they take effect.
    pub settings: ValidSettings,
}

/// The physiological half of a write that satisfied every bound.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidSettings {
    /// Instant the values take effect. Never in the future.
    pub valid_from: Timestamp,
    /// Weight in kilograms.
    pub weight_kg: f64,
    /// Height in centimetres.
    pub height_cm: f64,
    /// Sex, as the Watson equations distinguish it.
    pub sex: Sex,
    /// Birth date.
    pub birth_date: NaiveDate,
}

impl ValidSettings {
    /// Whether the four parameters of SPEC.md §10.0-D differ from `current`.
    ///
    /// `valid_from` is deliberately not compared: it says *when* a value applies,
    /// not what it is, and a request that repeats the values in force under a new
    /// effective date changes nothing about the profile.
    #[must_use]
    pub fn differs_from(&self, current: &ProfileSettings) -> bool {
        self.weight_kg != current.weight_kg
            || self.height_cm != current.height_cm
            || self.sex != current.sex
            || self.birth_date != current.birth_date
    }
}

/// Collects field errors while a request is read.
#[derive(Debug, Default)]
struct Report(Vec<FieldError>);

impl Report {
    fn add(&mut self, field: &str, detail: impl Into<String>) {
        self.0.push(FieldError::new(field, detail));
    }

    /// Reads a member that has to be there, reporting it by name when it is not.
    fn required<T>(&mut self, field: &str, value: Option<T>) -> Option<T> {
        if value.is_none() {
            self.add(field, "is required");
        }
        value
    }

    fn finish<T>(self, value: T) -> Result<T, ApiError> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(ApiError::validation(self.0))
        }
    }
}

/// Reads a finite number inside an exclusive range, or reports why it is not.
fn bounded(
    report: &mut Report,
    field: &str,
    value: Option<f64>,
    min_exclusive: f64,
    max_exclusive: f64,
) -> Option<f64> {
    let value = report.required(field, value)?;
    if value.is_finite() && value > min_exclusive && value < max_exclusive {
        return Some(value);
    }
    report.add(
        field,
        format!("must be a number strictly between {min_exclusive} and {max_exclusive}"),
    );
    None
}

impl ProfileRequest {
    /// Reads the request, reporting every offending member at once.
    ///
    /// `now` is passed in rather than read from the clock so that the two rules
    /// resting on it — a `valid_from` never in the future, a birth date never in
    /// the future — are the same rules under test as in production.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that cannot be used.
    pub fn validate(self, now: Timestamp) -> Result<ValidProfile, ApiError> {
        let mut report = Report::default();

        let display_name = report
            .required("display_name", self.display_name)
            .map(|name| name.trim().to_owned())
            .and_then(|name| {
                if name.is_empty() {
                    report.add("display_name", "must not be blank");
                    None
                } else {
                    Some(name)
                }
            });

        let unit = report.required("default_quantity_unit", self.default_quantity_unit);

        let ingestion = report
            .required(
                "default_ingestion_duration_seconds",
                self.default_ingestion_duration_seconds,
            )
            .and_then(|seconds| {
                if seconds >= INGESTION_DURATION_MIN_SECONDS {
                    Some(seconds)
                } else {
                    report.add(
                        "default_ingestion_duration_seconds",
                        format!("must be at least {INGESTION_DURATION_MIN_SECONDS} seconds"),
                    );
                    None
                }
            });

        let absorption = report
            .required(
                "default_absorption_duration_seconds",
                self.default_absorption_duration_seconds,
            )
            .and_then(|seconds| {
                if seconds > ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS {
                    Some(seconds)
                } else {
                    report.add(
                        "default_absorption_duration_seconds",
                        format!(
                            "must be more than {ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS} seconds"
                        ),
                    );
                    None
                }
            });

        let settings = report
            .required("settings", self.settings)
            .and_then(|settings| settings.read(&mut report, now));

        report.finish(())?;

        // Every `?` above having been cleared, each member is present.
        Ok(ValidProfile {
            display_name: display_name.expect("reported when absent"),
            default_quantity_unit: unit.expect("reported when absent"),
            default_ingestion_duration_seconds: ingestion.expect("reported when absent"),
            default_absorption_duration_seconds: absorption.expect("reported when absent"),
            settings: settings.expect("reported when absent"),
        })
    }
}

impl SettingsRequest {
    /// Reads the physiological half, prefixing every field with `settings.`.
    fn read(self, report: &mut Report, now: Timestamp) -> Option<ValidSettings> {
        let weight_kg = bounded(
            report,
            "settings.weight_kg",
            self.weight_kg,
            WEIGHT_KG_MIN_EXCLUSIVE,
            WEIGHT_KG_MAX_EXCLUSIVE,
        );
        let height_cm = bounded(
            report,
            "settings.height_cm",
            self.height_cm,
            HEIGHT_CM_MIN_EXCLUSIVE,
            HEIGHT_CM_MAX_EXCLUSIVE,
        );
        let sex = report.required("settings.sex", self.sex);
        let birth_date = report
            .required("settings.birth_date", self.birth_date)
            .and_then(|date| match now.date_naive().years_since(date) {
                None => {
                    report.add("settings.birth_date", "must not be in the future");
                    None
                }
                Some(age) if age > MAX_AGE_YEARS => {
                    report.add(
                        "settings.birth_date",
                        format!("must not imply an age above {MAX_AGE_YEARS} years"),
                    );
                    None
                }
                Some(_) => Some(date),
            });

        // A `valid_from` in the future would let a profile answer with parameters
        // it does not have yet (SPEC.md §5.1, §10.0-J).
        let valid_from = match self.valid_from {
            None => Some(now),
            Some(instant) if instant <= now => Some(instant),
            Some(_) => {
                report.add("settings.valid_from", "must not be in the future");
                None
            }
        };

        Some(ValidSettings {
            weight_kg: weight_kg?,
            height_cm: height_cm?,
            sex: sex?,
            birth_date: birth_date?,
            valid_from: valid_from?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether some migration carries `clause` verbatim.
    fn the_migrations_check(clause: &str) -> bool {
        db::MIGRATOR
            .migrations
            .iter()
            .any(|migration| migration.sql.contains(clause))
    }

    #[test]
    fn every_restated_bound_is_written_as_a_check_in_the_migrations() {
        // The four clauses are rebuilt from the constants, so moving a constant
        // without writing the matching migration fails here. It runs without a
        // database, which is the whole of its value.
        //
        // What it cannot say, stated so nobody reads more into it: **which**
        // clause is in force. It searches every migration, and migrations are
        // append-only — so the day a second one narrows a bound, the older text
        // is still here and a constant moved back to the older value would find
        // it and pass while the database went on refusing. There is one
        // migration today, so the two happen to coincide; that is a fact about
        // today, not a property of this test. The test that settles it asks the
        // live catalogue instead:
        // `the_schema_enforces_exactly_the_bounds_the_api_restates`, in
        // `crates/api/tests/database.rs`.
        for clause in [
            format!(
                "weight_kg > {WEIGHT_KG_MIN_EXCLUSIVE} AND weight_kg < {WEIGHT_KG_MAX_EXCLUSIVE}"
            ),
            format!(
                "height_cm > {HEIGHT_CM_MIN_EXCLUSIVE} AND height_cm < {HEIGHT_CM_MAX_EXCLUSIVE}"
            ),
            format!("default_ingestion_duration_seconds >= {INGESTION_DURATION_MIN_SECONDS}"),
            format!(
                "default_absorption_duration_seconds > {ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS}"
            ),
        ] {
            assert!(
                the_migrations_check(&clause),
                "no migration carries the CHECK `{clause}`"
            );
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a real instant")
    }

    fn born(text: &str) -> NaiveDate {
        text.parse().expect("a real date")
    }

    const NOW: &str = "2026-08-23T12:00:00Z";

    fn sound_settings() -> SettingsRequest {
        SettingsRequest {
            valid_from: None,
            weight_kg: Some(62.0),
            height_cm: Some(168.0),
            sex: Some(Sex::Female),
            birth_date: Some(born("1994-05-12")),
        }
    }

    fn sound_request() -> ProfileRequest {
        ProfileRequest {
            display_name: Some("  Alice  ".to_owned()),
            default_quantity_unit: Some(QuantityUnit::Cl),
            default_ingestion_duration_seconds: Some(1200),
            default_absorption_duration_seconds: Some(1800),
            settings: Some(sound_settings()),
        }
    }

    fn offending_fields(request: ProfileRequest) -> Vec<String> {
        request
            .validate(at(NOW))
            .expect_err("must be rejected")
            .to_problem_details()
            .errors
            .into_iter()
            .map(|error| error.field)
            .collect()
    }

    #[test]
    fn a_sound_request_is_accepted_and_its_name_is_trimmed() {
        let valid = sound_request().validate(at(NOW)).expect("must validate");
        assert_eq!(valid.display_name, "Alice");
        assert_eq!(valid.settings.weight_kg, 62.0);
        assert_eq!(valid.settings.sex, Sex::Female);
    }

    #[test]
    fn an_absent_valid_from_defaults_to_now() {
        let valid = sound_request().validate(at(NOW)).expect("must validate");
        assert_eq!(valid.settings.valid_from, at(NOW));
    }

    #[test]
    fn a_retroactive_valid_from_is_accepted() {
        // The gesture SPEC.md §5.1 exists to keep: "I had the wrong weight".
        let mut request = sound_request();
        let settings = SettingsRequest {
            valid_from: Some(at("2025-01-01T00:00:00Z")),
            ..sound_settings()
        };
        request.settings = Some(settings);
        let valid = request.validate(at(NOW)).expect("must validate");
        assert_eq!(valid.settings.valid_from, at("2025-01-01T00:00:00Z"));
    }

    #[test]
    fn a_valid_from_in_the_future_is_rejected() {
        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            valid_from: Some(at("2026-08-23T12:00:01Z")),
            ..sound_settings()
        });
        assert_eq!(offending_fields(request), vec!["settings.valid_from"]);
    }

    #[test]
    fn a_profile_without_a_sex_or_a_birth_date_is_rejected() {
        // Acceptance criterion of issue #6: without them the computation of
        // SPEC.md §6.2 has no branch and no age, so it cannot run at all.
        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            sex: None,
            birth_date: None,
            ..sound_settings()
        });
        assert_eq!(
            offending_fields(request),
            vec!["settings.sex", "settings.birth_date"]
        );
    }

    #[test]
    fn a_request_without_any_settings_names_the_member_it_wants() {
        let mut request = sound_request();
        request.settings = None;
        assert_eq!(offending_fields(request), vec!["settings"]);
    }

    #[test]
    fn every_offending_member_is_reported_in_one_answer() {
        // Reporting one at a time forces a client through as many round trips as
        // it made mistakes, which is what `errors[]` exists to avoid.
        let request = ProfileRequest {
            display_name: Some("   ".to_owned()),
            default_quantity_unit: None,
            default_ingestion_duration_seconds: Some(-1),
            default_absorption_duration_seconds: Some(0),
            settings: Some(SettingsRequest {
                valid_from: None,
                weight_kg: Some(0.0),
                height_cm: Some(1000.0),
                sex: None,
                birth_date: Some(born("2099-01-01")),
            }),
        };
        assert_eq!(
            offending_fields(request),
            vec![
                "display_name",
                "default_quantity_unit",
                "default_ingestion_duration_seconds",
                "default_absorption_duration_seconds",
                "settings.weight_kg",
                "settings.height_cm",
                "settings.sex",
                "settings.birth_date",
            ]
        );
    }

    #[test]
    fn a_bound_is_exclusive_on_both_ends() {
        for (weight, accepted) in [
            (WEIGHT_KG_MIN_EXCLUSIVE, false),
            (WEIGHT_KG_MAX_EXCLUSIVE, false),
            (WEIGHT_KG_MIN_EXCLUSIVE + 0.1, true),
            (WEIGHT_KG_MAX_EXCLUSIVE - 0.1, true),
        ] {
            let mut request = sound_request();
            request.settings = Some(SettingsRequest {
                weight_kg: Some(weight),
                ..sound_settings()
            });
            assert_eq!(
                request.validate(at(NOW)).is_ok(),
                accepted,
                "weight {weight} should {} be accepted",
                if accepted { "" } else { "not" }
            );
        }
    }

    #[test]
    fn an_instantaneous_ingestion_is_allowed_but_a_null_absorption_is_not() {
        // SPEC.md §6.4: a zero ingestion duration is the classic Widmark case; a
        // zero absorption duration degenerates the trapezoid.
        let mut request = sound_request();
        request.default_ingestion_duration_seconds = Some(0);
        assert!(request.validate(at(NOW)).is_ok());

        let mut request = sound_request();
        request.default_absorption_duration_seconds = Some(0);
        assert_eq!(
            offending_fields(request),
            vec!["default_absorption_duration_seconds"]
        );
    }

    #[test]
    fn a_birth_date_today_is_accepted_and_tomorrow_is_not() {
        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            birth_date: Some(born("2026-08-23")),
            ..sound_settings()
        });
        assert!(request.validate(at(NOW)).is_ok());

        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            birth_date: Some(born("2026-08-24")),
            ..sound_settings()
        });
        assert_eq!(offending_fields(request), vec!["settings.birth_date"]);
    }

    #[test]
    fn an_age_beyond_the_bound_is_rejected() {
        // A bound on human longevity, and only that: it refuses a birth date no
        // person could carry. It is *not* what keeps the male Watson equation of
        // SPEC.md §6.2 away from a non-positive total body water, and it does not
        // come close — measured, the equation crosses zero at 25.715 years at the
        // worst body this module accepts, 104 years inside this bound. Weight and
        // height are bounded away from zero and nothing more, on purpose, so that
        // no real body is turned away. See the module documentation, and #16 for
        // the guard that does close it.
        //
        // Both dates are derived from the constant, so moving it moves the test
        // with it instead of leaving two hand-written years behind.
        let today = at(NOW).date_naive();
        let exactly_at_the_bound = today - chrono::Months::new(MAX_AGE_YEARS * 12);
        let a_year_older = today - chrono::Months::new((MAX_AGE_YEARS + 1) * 12);

        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            birth_date: Some(exactly_at_the_bound),
            ..sound_settings()
        });
        assert!(
            request.validate(at(NOW)).is_ok(),
            "exactly {MAX_AGE_YEARS} years old must be accepted"
        );

        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            birth_date: Some(a_year_older),
            ..sound_settings()
        });
        assert_eq!(offending_fields(request), vec!["settings.birth_date"]);
    }

    #[test]
    fn a_non_finite_number_is_rejected_rather_than_bound_into_the_database() {
        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            weight_kg: Some(f64::INFINITY),
            ..sound_settings()
        });
        assert_eq!(offending_fields(request), vec!["settings.weight_kg"]);
    }

    #[test]
    fn a_message_quotes_the_bound_it_enforces() {
        // The messages are built from the constants, so raising a bound rewrites
        // them. Checked here because prose that spelled a number out would drift
        // in silence — the failure mode measured on #3.
        let mut request = sound_request();
        request.settings = Some(SettingsRequest {
            weight_kg: Some(WEIGHT_KG_MAX_EXCLUSIVE),
            ..sound_settings()
        });
        let problem = request
            .validate(at(NOW))
            .expect_err("must be rejected")
            .to_problem_details();
        assert!(
            problem.errors[0]
                .detail
                .contains(&WEIGHT_KG_MAX_EXCLUSIVE.to_string()),
            "{}",
            problem.errors[0].detail
        );
    }

    #[test]
    fn only_the_four_versioned_parameters_decide_whether_a_version_is_needed() {
        // SPEC.md §10.0-J: changing the input preferences alone creates no
        // version. `differs_from` is what the update handler asks, so a member
        // added to the comparison — or dropped from it — shows up here.
        let current = ProfileSettings {
            valid_from: at("2020-01-01T00:00:00Z"),
            weight_kg: 62.0,
            height_cm: 168.0,
            sex: Sex::Female,
            birth_date: born("1994-05-12"),
        };
        let unchanged = sound_request()
            .validate(at(NOW))
            .expect("must validate")
            .settings;
        assert!(
            !unchanged.differs_from(&current),
            "an unchanged set of parameters must not ask for a version"
        );

        for changed in [
            SettingsRequest {
                weight_kg: Some(63.0),
                ..sound_settings()
            },
            SettingsRequest {
                height_cm: Some(169.0),
                ..sound_settings()
            },
            SettingsRequest {
                sex: Some(Sex::Male),
                ..sound_settings()
            },
            SettingsRequest {
                birth_date: Some(born("1994-05-13")),
                ..sound_settings()
            },
        ] {
            let mut request = sound_request();
            request.settings = Some(changed);
            let settings = request.validate(at(NOW)).expect("must validate").settings;
            assert!(settings.differs_from(&current));
        }
    }
}
