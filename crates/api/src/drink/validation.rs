//! Reading a drink write: defaults, merge of a modification, and the rules.
//!
//! The rules of a drink itself — a name, one or more components, the total
//! volume mandatory in percent and refused in cL, shares summing to at most
//! 100 %, durations within the schema's bounds — are those of
//! `domain::drink`, applied here and nowhere else in this crate, so that the
//! API and the database check (migration `20261021090000`) refuse the same
//! drinks. What this module adds is what only an HTTP payload has: missing
//! members, server-side defaults, the feeling rating, and the merge of a
//! `PATCH` onto the stored drink.

use domain::drink::{ComponentDraft, DrinkDraft, DrinkError};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};
use crate::profile::QuantityUnit;
use crate::timestamp::Timestamp;

use super::model::{ComponentRequest, Drink, DrinkPatch, DrinkRequest};

/// Lowest feeling rating (SPEC.md §5.9), as the schema's `CHECK` spells it.
pub const FEELING_RATING_MIN: i64 = 1;

/// Highest feeling rating (SPEC.md §5.9), as the schema's `CHECK` spells it.
pub const FEELING_RATING_MAX: i64 = 10;

/// The earliest instant a `timestamptz` holds: 4714-11-24 00:00:00 UTC BC,
/// that is year −4713 in the proleptic Gregorian calendar `chrono` counts in.
/// `chrono` reads earlier instants, which PostgreSQL then refuses with a
/// 500; the database test `an_ingestion_time_is_held_to_what_the_database_stores`
/// ties this value to the live server, in both directions.
#[must_use]
pub fn earliest_ingested_at() -> Timestamp {
    chrono::NaiveDate::from_ymd_opt(-4713, 11, 24)
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .expect("a real instant")
        .and_utc()
}

/// The input preferences of a profile a creation falls back on (SPEC.md §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// Ingestion duration used when the request names none, in seconds.
    pub ingestion_duration_seconds: i32,
    /// Absorption duration used when the request names none, in seconds.
    pub absorption_duration_seconds: i32,
}

/// A write that satisfied every rule, ready to be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidDrink {
    /// Event the drink is attached to, if any.
    pub event_id: Option<Uuid>,
    /// Instant the drink started being drunk.
    pub ingested_at: Timestamp,
    /// Feeling rating, within the scale.
    pub feeling_rating: Option<i16>,
    /// The drink itself, validated by the domain, its quantities in the
    /// domain's units: millilitres in cL, percent in percent.
    pub drink: domain::drink::Drink,
    /// The component quantities as the request expressed them — centilitres
    /// in cL, percent in percent — in the order of the components. They are
    /// what is stored and served back, so that a quantity round-trips to the
    /// bit: `x * 10 / 10` does not give `x` back for every `f64` (839.48 does
    /// not), so the millilitres of [`ValidDrink::drink`] cannot stand in.
    pub quantities: Vec<f64>,
}

/// Millilitres in a centilitre: a cL quantity reaches the domain, which
/// counts volumes in millilitres, multiplied by this.
pub const ML_PER_CL: f64 = 10.0;

/// A component quantity as the API carries it — centilitres in cL, percent in
/// percent — in the unit of the domain: millilitres in cL, percent in percent.
/// The one place the conversion is written, for a write and for a stored
/// drink read back alike.
fn domain_quantity(unit: QuantityUnit, quantity: f64) -> f64 {
    match unit {
        QuantityUnit::Cl => quantity * ML_PER_CL,
        QuantityUnit::Percent => quantity,
    }
}

impl From<QuantityUnit> for domain::drink::QuantityUnit {
    fn from(unit: QuantityUnit) -> Self {
        match unit {
            QuantityUnit::Cl => Self::Cl,
            QuantityUnit::Percent => Self::Percent,
        }
    }
}

impl From<domain::drink::QuantityUnit> for QuantityUnit {
    fn from(unit: domain::drink::QuantityUnit) -> Self {
        match unit {
            domain::drink::QuantityUnit::Cl => Self::Cl,
            domain::drink::QuantityUnit::Percent => Self::Percent,
        }
    }
}

/// Reads a text member, trimmed, reporting it when it is missing or carries a
/// NUL character, which PostgreSQL refuses in `text` — unreported here, it
/// would come back from the database as a 500.
fn text(errors: &mut Vec<FieldError>, field: &str, value: Option<String>) -> Option<String> {
    let Some(value) = value else {
        errors.push(FieldError::new(field, "is required"));
        return None;
    };
    if value.contains('\0') {
        errors.push(FieldError::new(field, "must not contain a NUL character"));
        return None;
    }
    Some(value.trim().to_owned())
}

/// Reads a `PATCH` member that cannot be cleared: absent is `None`, an
/// explicit `null` is reported and read as absent.
fn not_null<T>(errors: &mut Vec<FieldError>, field: &str, value: Option<Option<T>>) -> Option<T> {
    match value {
        Some(None) => {
            errors.push(FieldError::new(field, "must not be null"));
            None
        }
        Some(Some(value)) => Some(value),
        None => None,
    }
}

impl ComponentRequest {
    /// Reads a component, prefixing every field with its position. `None` when
    /// a member is missing or unusable; the rules of the domain run afterwards.
    fn read(self, errors: &mut Vec<FieldError>, index: usize) -> Option<ComponentDraft> {
        let name = text(errors, &format!("components[{index}].name"), self.name);
        let mut required = |field: &str, value: Option<f64>| {
            if value.is_none() {
                errors.push(FieldError::new(
                    format!("components[{index}].{field}"),
                    "is required",
                ));
            }
            value
        };
        let abv_percent = required("abv_percent", self.abv_percent);
        let quantity = required("quantity", self.quantity);
        Some(ComponentDraft {
            name: name?,
            abv_percent: abv_percent?,
            quantity: quantity?,
        })
    }
}

impl DrinkRequest {
    /// Reads a creation, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that cannot be used.
    pub fn validate(self, defaults: Defaults, now: Timestamp) -> Result<ValidDrink, ApiError> {
        let mut errors = Vec::new();

        let name = text(&mut errors, "name", self.name);
        if name.as_deref().is_some_and(str::is_empty) {
            errors.push(FieldError::new("name", "must not be blank"));
        }

        let unit = self.quantity_unit;
        if unit.is_none() {
            errors.push(FieldError::new("quantity_unit", "is required"));
        }

        let ingested_at = self.ingested_at.unwrap_or(now);
        if ingested_at < earliest_ingested_at() {
            errors.push(FieldError::new(
                "ingested_at",
                format!("must not be earlier than {}", earliest_ingested_at()),
            ));
        }

        let mut components = Vec::new();
        let mut complete = true;
        for (index, component) in self.components.unwrap_or_default().into_iter().enumerate() {
            match component.read(&mut errors, index) {
                Some(draft) => components.push(draft),
                None => complete = false,
            }
        }

        let feeling_rating = match self.feeling_rating {
            None => None,
            Some(rating) => match i16::try_from(rating) {
                Ok(rating)
                    if (FEELING_RATING_MIN..=FEELING_RATING_MAX).contains(&i64::from(rating)) =>
                {
                    Some(rating)
                }
                _ => {
                    errors.push(FieldError::new(
                        "feeling_rating",
                        format!("must be between {FEELING_RATING_MIN} and {FEELING_RATING_MAX}"),
                    ));
                    None
                }
            },
        };

        // The rules of the domain are only meaningful on a complete drink: the
        // unit decides how every quantity reads, and a component missing a
        // member has nothing to check yet.
        let quantities: Vec<f64> = components.iter().map(|c| c.quantity).collect();
        let drink = match unit {
            Some(unit) if complete => DrinkDraft {
                name: name.unwrap_or_default(),
                unit: unit.into(),
                volume_total_ml: self.volume_total_ml,
                ingestion_duration_seconds: self
                    .ingestion_duration_seconds
                    .unwrap_or_else(|| i64::from(defaults.ingestion_duration_seconds)),
                absorption_duration_seconds: self
                    .absorption_duration_seconds
                    .unwrap_or_else(|| i64::from(defaults.absorption_duration_seconds)),
                components: components
                    .into_iter()
                    .map(|component| ComponentDraft {
                        quantity: domain_quantity(unit, component.quantity),
                        ..component
                    })
                    .collect(),
            }
            .validate()
            .map_err(|refused| {
                // A blank name is already reported above, with the missing one.
                errors.extend(
                    refused
                        .into_iter()
                        .filter(|error| *error != DrinkError::BlankName)
                        .map(|error| FieldError::new(error.field(), error.to_string())),
                );
            })
            .ok(),
            _ => None,
        };

        match drink {
            Some(drink) if errors.is_empty() => Ok(ValidDrink {
                event_id: self.event_id,
                ingested_at,
                feeling_rating,
                drink,
                quantities,
            }),
            _ => Err(ApiError::validation(errors)),
        }
    }
}

impl Drink {
    /// The stored drink as the domain counts it, for a computation: its
    /// quantities converted to millilitres in cL, as a write converts them.
    ///
    /// # Errors
    ///
    /// The rules of the domain the stored drink breaks — none for a drink the
    /// API and the database both accepted.
    pub fn to_domain(&self) -> Result<domain::drink::Drink, Vec<DrinkError>> {
        DrinkDraft {
            name: self.name.clone(),
            unit: self.quantity_unit.into(),
            volume_total_ml: self.volume_total_ml,
            ingestion_duration_seconds: i64::from(self.ingestion_duration_seconds),
            absorption_duration_seconds: i64::from(self.absorption_duration_seconds),
            components: self
                .components
                .iter()
                .map(|component| ComponentDraft {
                    name: component.name.clone(),
                    abv_percent: component.abv_percent,
                    quantity: domain_quantity(self.quantity_unit, component.quantity),
                })
                .collect(),
        }
        .validate()
    }
}

impl DrinkPatch {
    /// Applies a modification onto the stored drink and reads the result as a
    /// whole, with the rules of a creation.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that cannot be used.
    pub fn merge_onto(self, stored: Drink) -> Result<ValidDrink, ApiError> {
        // A `null` on a member that cannot be cleared is refused; the member
        // is then read as absent, so that the rest is still reported.
        let mut refused = Vec::new();
        let name = not_null(&mut refused, "name", self.name);
        let ingested_at = not_null(&mut refused, "ingested_at", self.ingested_at);
        let ingestion = not_null(
            &mut refused,
            "ingestion_duration_seconds",
            self.ingestion_duration_seconds,
        );
        let absorption = not_null(
            &mut refused,
            "absorption_duration_seconds",
            self.absorption_duration_seconds,
        );
        let quantity_unit = not_null(&mut refused, "quantity_unit", self.quantity_unit);
        let components_sent = matches!(self.components, Some(Some(_)));
        let components = not_null(&mut refused, "components", self.components);

        let unit_changes = quantity_unit.is_some_and(|unit| unit != stored.quantity_unit);
        if unit_changes && !components_sent && !refused.iter().any(|e| e.field == "components") {
            // The stored quantities read differently in the other unit: 4 cL
            // of vodka would become 4 % of the drink, silently.
            refused.push(FieldError::new(
                "components",
                "must be sent again when quantity_unit changes",
            ));
        }

        let defaults = Defaults {
            ingestion_duration_seconds: stored.ingestion_duration_seconds,
            absorption_duration_seconds: stored.absorption_duration_seconds,
        };
        let merged = DrinkRequest {
            event_id: self.event_id.unwrap_or(stored.event_id),
            name: Some(name.unwrap_or(stored.name)),
            ingested_at: Some(ingested_at.unwrap_or(stored.ingested_at)),
            ingestion_duration_seconds: ingestion,
            absorption_duration_seconds: absorption,
            quantity_unit: Some(quantity_unit.unwrap_or(stored.quantity_unit)),
            volume_total_ml: self.volume_total_ml.unwrap_or(stored.volume_total_ml),
            feeling_rating: self
                .feeling_rating
                .unwrap_or(stored.feeling_rating.map(i64::from)),
            components: Some(components.unwrap_or_else(|| {
                stored
                    .components
                    .into_iter()
                    .map(ComponentRequest::from)
                    .collect()
            })),
        };
        let result = merged.validate(defaults, stored.ingested_at);

        if refused.is_empty() {
            return result;
        }
        // What the rules find wrong with a member already refused here — the
        // stored components read in the other unit, say — is beside the point.
        let mut errors: Vec<FieldError> = match result {
            Err(ApiError::Validation(errors)) => errors
                .into_iter()
                .filter(|error| {
                    !refused
                        .iter()
                        .any(|mine| error.field.starts_with(&mine.field))
                })
                .collect(),
            _ => Vec::new(),
        };
        errors.extend(refused);
        Err(ApiError::validation(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drink::model::Component;

    fn now() -> Timestamp {
        "2026-10-20T21:00:00Z".parse().expect("a real instant")
    }

    const DEFAULTS: Defaults = Defaults {
        ingestion_duration_seconds: 1200,
        absorption_duration_seconds: 1800,
    };

    fn component(name: &str, abv_percent: f64, quantity: f64) -> ComponentRequest {
        ComponentRequest {
            name: Some(name.to_owned()),
            abv_percent: Some(abv_percent),
            quantity: Some(quantity),
        }
    }

    fn a_beer() -> DrinkRequest {
        DrinkRequest {
            name: Some("Bière blonde".to_owned()),
            quantity_unit: Some(QuantityUnit::Cl),
            components: Some(vec![component("Bière blonde", 5.0, 25.0)]),
            ..DrinkRequest::default()
        }
    }

    /// The fields a validation failure names, sorted: which member is
    /// reported matters, the order of the report does not.
    fn fields_of(result: Result<ValidDrink, ApiError>) -> Vec<String> {
        match result {
            Err(ApiError::Validation(errors)) => {
                let mut fields: Vec<String> = errors.into_iter().map(|e| e.field).collect();
                fields.sort();
                fields
            }
            other => panic!("expected a validation failure, got {other:?}"),
        }
    }

    fn stored() -> Drink {
        Drink {
            id: Uuid::nil(),
            profile_id: Uuid::nil(),
            event_id: Some(Uuid::max()),
            name: "Screwdriver".to_owned(),
            ingested_at: "2026-10-19T22:00:00Z".parse().expect("a real instant"),
            ingestion_duration_seconds: 900,
            absorption_duration_seconds: 2400,
            quantity_unit: QuantityUnit::Cl,
            volume_total_ml: None,
            feeling_rating: Some(3),
            components: vec![
                Component {
                    name: "Vodka".to_owned(),
                    abv_percent: 40.0,
                    quantity: 40.0,
                },
                Component {
                    name: "Jus d'orange".to_owned(),
                    abv_percent: 0.0,
                    quantity: 160.0,
                },
            ],
            created_at: "2026-10-19T22:00:01Z".parse().expect("a real instant"),
            updated_at: "2026-10-19T22:00:01Z".parse().expect("a real instant"),
        }
    }

    fn component_names(valid: &ValidDrink) -> Vec<&str> {
        valid
            .drink
            .components()
            .iter()
            .map(|c| c.name.as_str())
            .collect()
    }

    #[test]
    fn the_ingestion_time_and_the_durations_default_to_now_and_the_profile() {
        let valid = a_beer().validate(DEFAULTS, now()).expect("valid");
        assert_eq!(valid.ingested_at, now());
        assert_eq!(valid.drink.ingestion_duration_seconds(), 1200);
        assert_eq!(valid.drink.absorption_duration_seconds(), 1800);
        assert_eq!(valid.event_id, None);
        assert_eq!(valid.feeling_rating, None);
    }

    #[test]
    fn values_the_request_names_win_over_the_defaults() {
        let at: Timestamp = "2026-10-20T20:15:00Z".parse().expect("a real instant");
        let event = Uuid::max();
        let valid = DrinkRequest {
            event_id: Some(event),
            ingested_at: Some(at),
            ingestion_duration_seconds: Some(0),
            absorption_duration_seconds: Some(60),
            feeling_rating: Some(7),
            ..a_beer()
        }
        .validate(DEFAULTS, now())
        .expect("valid");
        assert_eq!(valid.ingested_at, at);
        assert_eq!(valid.drink.ingestion_duration_seconds(), 0);
        assert_eq!(valid.drink.absorption_duration_seconds(), 60);
        assert_eq!(valid.event_id, Some(event));
        assert_eq!(valid.feeling_rating, Some(7));
    }

    #[test]
    fn names_are_trimmed() {
        let valid = DrinkRequest {
            name: Some("  Panaché ".to_owned()),
            components: Some(vec![component(" Bière ", 5.0, 125.0)]),
            ..a_beer()
        }
        .validate(DEFAULTS, now())
        .expect("valid");
        assert_eq!(valid.drink.name(), "Panaché");
        assert_eq!(component_names(&valid), ["Bière"]);
    }

    #[test]
    fn a_missing_or_blank_name_is_reported() {
        for name in [None, Some("   ".to_owned())] {
            let request = DrinkRequest { name, ..a_beer() };
            assert_eq!(fields_of(request.validate(DEFAULTS, now())), ["name"]);
        }
    }

    #[test]
    fn the_unit_has_no_default() {
        let request = DrinkRequest {
            quantity_unit: None,
            ..a_beer()
        };
        assert_eq!(
            fields_of(request.validate(DEFAULTS, now())),
            ["quantity_unit"]
        );
    }

    #[test]
    fn a_drink_without_components_is_reported() {
        for components in [None, Some(Vec::new())] {
            let request = DrinkRequest {
                components,
                ..a_beer()
            };
            assert_eq!(fields_of(request.validate(DEFAULTS, now())), ["components"]);
        }
    }

    #[test]
    fn a_component_missing_a_member_is_reported_by_position() {
        let request = DrinkRequest {
            components: Some(vec![
                component("Gin", 40.0, 40.0),
                ComponentRequest {
                    name: Some("Tonic".to_owned()),
                    ..ComponentRequest::default()
                },
            ]),
            ..a_beer()
        };
        assert_eq!(
            fields_of(request.validate(DEFAULTS, now())),
            ["components[1].abv_percent", "components[1].quantity"]
        );
    }

    #[test]
    fn the_rules_of_the_domain_are_reported_under_their_fields() {
        let percent_without_total = DrinkRequest {
            quantity_unit: Some(QuantityUnit::Percent),
            components: Some(vec![component("Bière", 5.0, 100.0)]),
            ..a_beer()
        };
        assert_eq!(
            fields_of(percent_without_total.validate(DEFAULTS, now())),
            ["volume_total_ml"]
        );

        let cl_with_total = DrinkRequest {
            volume_total_ml: Some(250.0),
            ..a_beer()
        };
        assert_eq!(
            fields_of(cl_with_total.validate(DEFAULTS, now())),
            ["volume_total_ml"]
        );

        let above_100 = DrinkRequest {
            quantity_unit: Some(QuantityUnit::Percent),
            volume_total_ml: Some(200.0),
            components: Some(vec![
                component("Gin", 40.0, 60.0),
                component("Tonic", 0.0, 50.0),
            ]),
            ..a_beer()
        };
        assert_eq!(
            fields_of(above_100.validate(DEFAULTS, now())),
            ["components"]
        );

        let bad_durations = DrinkRequest {
            ingestion_duration_seconds: Some(-1),
            absorption_duration_seconds: Some(i64::from(i32::MAX) + 1),
            ..a_beer()
        };
        assert_eq!(
            fields_of(bad_durations.validate(DEFAULTS, now())),
            ["absorption_duration_seconds", "ingestion_duration_seconds"]
        );

        let bad_component = DrinkRequest {
            components: Some(vec![component("Bière", 100.5, 250.0)]),
            ..a_beer()
        };
        assert_eq!(
            fields_of(bad_component.validate(DEFAULTS, now())),
            ["components[0].abv_percent"]
        );
    }

    #[test]
    fn the_feeling_rating_stays_on_its_scale() {
        for rating in [FEELING_RATING_MIN, FEELING_RATING_MAX] {
            let valid = DrinkRequest {
                feeling_rating: Some(rating),
                ..a_beer()
            }
            .validate(DEFAULTS, now())
            .expect("a rating on the scale is valid");
            assert_eq!(valid.feeling_rating.map(i64::from), Some(rating));
        }
        for rating in [FEELING_RATING_MIN - 1, FEELING_RATING_MAX + 1, i64::MAX] {
            let request = DrinkRequest {
                feeling_rating: Some(rating),
                ..a_beer()
            };
            assert_eq!(
                fields_of(request.validate(DEFAULTS, now())),
                ["feeling_rating"],
                "{rating}"
            );
        }
    }

    #[test]
    fn an_ingestion_time_before_what_the_database_stores_is_a_field_error() {
        let earliest = earliest_ingested_at();
        let valid = DrinkRequest {
            ingested_at: Some(earliest),
            ..a_beer()
        }
        .validate(DEFAULTS, now())
        .expect("the earliest storable instant is valid");
        assert_eq!(valid.ingested_at, earliest);

        for too_early in [
            earliest - chrono::Duration::microseconds(1),
            "-5000-01-01T00:00:00Z".parse().expect("chrono reads it"),
        ] {
            let request = DrinkRequest {
                ingested_at: Some(too_early),
                ..a_beer()
            };
            assert_eq!(
                fields_of(request.validate(DEFAULTS, now())),
                ["ingested_at"],
                "{too_early}"
            );
            let patch = DrinkPatch {
                ingested_at: Some(Some(too_early)),
                ..DrinkPatch::default()
            };
            assert_eq!(fields_of(patch.merge_onto(stored())), ["ingested_at"]);
        }
    }

    #[test]
    fn a_nul_character_is_a_field_error_and_not_a_server_failure() {
        // PostgreSQL refuses a NUL inside `text`: unreported here, it would
        // reach the database and come back as a 500.
        let request = DrinkRequest {
            name: Some("Bi\0ère".to_owned()),
            components: Some(vec![component("Bi\0ère", 5.0, 250.0)]),
            ..a_beer()
        };
        assert_eq!(
            fields_of(request.validate(DEFAULTS, now())),
            ["components[0].name", "name"]
        );
    }

    #[test]
    fn every_offending_member_is_reported_at_once() {
        let request = DrinkRequest {
            name: None,
            quantity_unit: None,
            feeling_rating: Some(0),
            ..a_beer()
        };
        assert_eq!(
            fields_of(request.validate(DEFAULTS, now())),
            ["feeling_rating", "name", "quantity_unit"]
        );
    }

    #[test]
    fn an_empty_patch_keeps_the_stored_drink() {
        let valid = DrinkPatch::default().merge_onto(stored()).expect("valid");
        assert_eq!(valid.event_id, Some(Uuid::max()));
        assert_eq!(valid.ingested_at, stored().ingested_at);
        assert_eq!(valid.feeling_rating, Some(3));
        assert_eq!(valid.drink.name(), "Screwdriver");
        assert_eq!(valid.drink.ingestion_duration_seconds(), 900);
        assert_eq!(valid.drink.absorption_duration_seconds(), 2400);
        assert_eq!(valid.drink.unit(), domain::drink::QuantityUnit::Cl);
        assert_eq!(valid.drink.entered_volume_total_ml(), None);
        assert_eq!(component_names(&valid), ["Vodka", "Jus d'orange"]);
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let at: Timestamp = "2026-10-19T22:30:00Z".parse().expect("a real instant");
        let valid = DrinkPatch {
            name: Some(Some("Vodka orange".to_owned())),
            ingested_at: Some(Some(at)),
            absorption_duration_seconds: Some(Some(600)),
            ..DrinkPatch::default()
        }
        .merge_onto(stored())
        .expect("valid");
        assert_eq!(valid.drink.name(), "Vodka orange");
        assert_eq!(valid.ingested_at, at);
        assert_eq!(valid.drink.absorption_duration_seconds(), 600);
        assert_eq!(valid.drink.ingestion_duration_seconds(), 900);
        assert_eq!(valid.feeling_rating, Some(3));
        assert_eq!(component_names(&valid), ["Vodka", "Jus d'orange"]);
    }

    #[test]
    fn a_null_on_a_member_that_cannot_be_cleared_is_a_field_error() {
        // Read as "absent", it would answer 200 on a request asking for a
        // drink without a name or without a time, and keep the old value.
        for member in [
            "name",
            "ingested_at",
            "ingestion_duration_seconds",
            "absorption_duration_seconds",
            "quantity_unit",
            "components",
        ] {
            let patch: DrinkPatch =
                serde_json::from_str(&format!(r#"{{"{member}":null}}"#)).expect("must parse");
            assert_eq!(fields_of(patch.merge_onto(stored())), [member], "{member}");
        }
    }

    #[test]
    fn a_partial_patch_keeps_the_total_volume_of_a_drink_in_percent() {
        // The stored volume is the only thing that makes the shares
        // computable: dropping it would turn a rename into a refusal.
        let mut in_percent = stored();
        in_percent.quantity_unit = QuantityUnit::Percent;
        in_percent.volume_total_ml = Some(200.0);
        in_percent.components[0].quantity = 20.0;
        in_percent.components[1].quantity = 80.0;
        let valid = DrinkPatch {
            name: Some(Some("Vodka orange".to_owned())),
            ..DrinkPatch::default()
        }
        .merge_onto(in_percent)
        .expect("valid");
        assert_eq!(valid.drink.unit(), domain::drink::QuantityUnit::Percent);
        assert_eq!(valid.drink.entered_volume_total_ml(), Some(200.0));
    }

    #[test]
    fn moving_a_drink_from_percent_to_cl_takes_a_null_total_volume() {
        let mut in_percent = stored();
        in_percent.quantity_unit = QuantityUnit::Percent;
        in_percent.volume_total_ml = Some(200.0);
        in_percent.components[0].quantity = 20.0;
        in_percent.components[1].quantity = 80.0;
        let to_cl = |volume_total_ml| DrinkPatch {
            quantity_unit: Some(Some(QuantityUnit::Cl)),
            volume_total_ml,
            components: Some(Some(vec![component("Vodka", 40.0, 4.0)])),
            ..DrinkPatch::default()
        };

        let valid = to_cl(Some(None))
            .merge_onto(in_percent.clone())
            .expect("null clears the total a drink in cL may not carry");
        assert_eq!(valid.drink.unit(), domain::drink::QuantityUnit::Cl);
        assert_eq!(valid.drink.entered_volume_total_ml(), None);
        assert_eq!(valid.quantities, [4.0]);
        assert_eq!(valid.drink.component_volumes_ml(), [40.0]);

        // Absent, the stored total stays, and a drink in cL refuses it.
        assert_eq!(
            fields_of(to_cl(None).merge_onto(in_percent)),
            ["volume_total_ml"]
        );
    }

    #[test]
    fn a_cl_quantity_reaches_the_domain_in_millilitres() {
        // The user decision on #14: cL on the wire. The domain counts in
        // millilitres, so 25 cL of beer at 5 % is 250 mL of it.
        let valid = a_beer().validate(DEFAULTS, now()).expect("valid");
        assert_eq!(valid.drink.component_volumes_ml(), [250.0]);
        assert_eq!(
            valid.drink.pure_alcohol_grams().to_bits(),
            domain::ingested_alcohol_grams(250.0, 5.0).to_bits()
        );
        assert_eq!(valid.quantities, [25.0]);

        // A share is a share: nothing is converted in percent.
        let valid = DrinkRequest {
            quantity_unit: Some(QuantityUnit::Percent),
            volume_total_ml: Some(200.0),
            components: Some(vec![
                component("Gin", 40.0, 25.0),
                component("Tonic", 0.0, 75.0),
            ]),
            ..a_beer()
        }
        .validate(DEFAULTS, now())
        .expect("valid");
        assert_eq!(valid.drink.component_volumes_ml(), [50.0, 150.0]);
        assert_eq!(valid.quantities, [25.0, 75.0]);
    }

    #[test]
    fn the_quantities_stored_are_the_ones_entered_to_the_bit() {
        // 839.48 * 10 / 10 is not 839.48 in f64: storing the millilitres
        // would serve back a different number than the one written.
        assert_ne!(
            (839.48_f64 * ML_PER_CL / ML_PER_CL).to_bits(),
            839.48_f64.to_bits()
        );
        let entered = [839.48, 0.1, 117.217];
        let valid = DrinkRequest {
            components: Some(
                entered
                    .iter()
                    .map(|quantity| component("Bière", 5.0, *quantity))
                    .collect(),
            ),
            ..a_beer()
        }
        .validate(DEFAULTS, now())
        .expect("valid");
        let bits = |values: &[f64]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&valid.quantities), bits(&entered));
    }

    #[test]
    fn new_components_replace_the_whole_composition() {
        let valid = DrinkPatch {
            components: Some(Some(vec![component("Vodka", 40.0, 50.0)])),
            ..DrinkPatch::default()
        }
        .merge_onto(stored())
        .expect("valid");
        assert_eq!(component_names(&valid), ["Vodka"]);
        assert_eq!(valid.quantities, [50.0]);
    }

    #[test]
    fn an_explicit_null_clears_and_an_absent_member_keeps() {
        let valid = DrinkPatch {
            feeling_rating: Some(None),
            event_id: Some(None),
            ..DrinkPatch::default()
        }
        .merge_onto(stored())
        .expect("valid");
        assert_eq!(valid.feeling_rating, None);
        assert_eq!(valid.event_id, None);
    }

    #[test]
    fn changing_the_unit_requires_the_components_again() {
        // A stored 40 cL of vodka would silently become 40 % of the drink.
        // The quantities are chosen to be valid shares too (40 + 50 ≤ 100),
        // so that only this rule can refuse them.
        let mut small = stored();
        small.components[1].quantity = 50.0;
        let patch = DrinkPatch {
            quantity_unit: Some(Some(QuantityUnit::Percent)),
            volume_total_ml: Some(Some(200.0)),
            ..DrinkPatch::default()
        };
        assert_eq!(fields_of(patch.clone().merge_onto(small)), ["components"]);
        // Read as shares, 40 + 160 exceed 100 %: the rule is reported alone,
        // not with what the domain finds wrong in a reading nobody asked for.
        assert_eq!(fields_of(patch.merge_onto(stored())), ["components"]);

        let valid = DrinkPatch {
            quantity_unit: Some(Some(QuantityUnit::Percent)),
            volume_total_ml: Some(Some(200.0)),
            components: Some(Some(vec![
                component("Vodka", 40.0, 20.0),
                component("Jus d'orange", 0.0, 80.0),
            ])),
            ..DrinkPatch::default()
        }
        .merge_onto(stored())
        .expect("valid");
        assert_eq!(valid.drink.unit(), domain::drink::QuantityUnit::Percent);
        assert_eq!(valid.drink.entered_volume_total_ml(), Some(200.0));

        // Naming the unit already stored is no change.
        DrinkPatch {
            quantity_unit: Some(Some(QuantityUnit::Cl)),
            ..DrinkPatch::default()
        }
        .merge_onto(stored())
        .expect("the same unit is no change of unit");
    }

    #[test]
    fn a_stored_drink_in_cl_reaches_the_domain_in_millilitres() {
        // 40 cL of vodka and 160 cL of orange juice: 400 mL and 1600 mL.
        let drink = stored().to_domain().expect("a stored drink is valid");
        assert_eq!(drink.component_volumes_ml(), [400.0, 1600.0]);
        assert_eq!(drink.unit(), domain::drink::QuantityUnit::Cl);
        assert_eq!(drink.ingestion_duration_seconds(), 900);
        assert_eq!(drink.absorption_duration_seconds(), 2400);
        // The very drink a write builds from the same quantities.
        let written = DrinkPatch::default()
            .merge_onto(stored())
            .expect("valid")
            .drink;
        assert_eq!(drink, written);
    }

    #[test]
    fn a_stored_drink_in_percent_keeps_its_shares() {
        let mut shares = stored();
        shares.quantity_unit = QuantityUnit::Percent;
        shares.volume_total_ml = Some(200.0);
        shares.components[0].quantity = 20.0;
        shares.components[1].quantity = 80.0;
        let drink = shares.to_domain().expect("a stored drink is valid");
        assert_eq!(drink.component_volumes_ml(), [40.0, 160.0]);
        assert_eq!(drink.entered_volume_total_ml(), Some(200.0));
    }

    #[test]
    fn a_stored_drink_breaking_a_rule_is_reported() {
        let mut empty = stored();
        empty.components.clear();
        assert_eq!(empty.to_domain(), Err(vec![DrinkError::NoComponent]));
    }

    #[test]
    fn a_patch_is_held_to_the_rules_of_a_creation() {
        let patch = DrinkPatch {
            volume_total_ml: Some(Some(200.0)),
            name: Some(Some(" ".to_owned())),
            feeling_rating: Some(Some(11)),
            ..DrinkPatch::default()
        };
        assert_eq!(
            fields_of(patch.merge_onto(stored())),
            ["feeling_rating", "name", "volume_total_ml"]
        );
    }
}
