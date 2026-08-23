//! Payload types of the profile endpoints.
//!
//! The split of the payload mirrors the split of the schema, and it is the point
//! of this module: identity and input preferences sit at the top level, the four
//! physiological parameters sit under `settings` because they live in
//! `profile_settings_version` and nowhere else (SPEC.md §4, §5.1, §10.0-L). A
//! reader of the payload can see which half is versioned without opening the
//! schema.
//!
//! `settings` is **nullable in the answer**. `profile_settings_at()` returns zero
//! rows rather than a row of `NULL`s when a profile owns no version, and SPEC.md
//! §10.0-L class (d) says that state is reachable under concurrent writes for as
//! long as #43 is open. Rendering it as `null` is the honest reading; inventing
//! defaults would be the dishonest one.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::timestamp::Timestamp;

/// Sex, as the Watson equations distinguish it (SPEC.md §6.2).
///
/// The two variants are the two branches of the model, and they are also the two
/// labels of the `sex` enum of the database — [`Sex::as_sql`] is the single
/// place that spelling is written, and a test ties it to the migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    /// Male branch of the Watson equations, the one carrying an age term.
    Male,
    /// Female branch of the Watson equations.
    Female,
}

impl Sex {
    /// Every variant, in the order the database enum declares them.
    pub const ALL: [Self; 2] = [Self::Male, Self::Female];

    /// Label of this variant in the PostgreSQL `sex` enum.
    #[must_use]
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }

    /// Reads back a label produced by [`Sex::as_sql`].
    #[must_use]
    pub fn from_sql(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_sql() == label)
    }
}

/// Unit a quantity is entered in (SPEC.md §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuantityUnit {
    /// Centilitres.
    Cl,
    /// Percentage of the total volume of the drink.
    Percent,
}

impl QuantityUnit {
    /// Every variant, in the order the database enum declares them.
    pub const ALL: [Self; 2] = [Self::Cl, Self::Percent];

    /// Label of this variant in the PostgreSQL `quantity_unit` enum.
    #[must_use]
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Cl => "cl",
            Self::Percent => "percent",
        }
    }

    /// Reads back a label produced by [`QuantityUnit::as_sql`].
    #[must_use]
    pub fn from_sql(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_sql() == label)
    }
}

/// The physiological parameters in force at the instant the answer was built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProfileSettings {
    /// Instant this version took effect (SPEC.md §10.0-J).
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub valid_from: Timestamp,
    /// Weight in kilograms.
    pub weight_kg: f64,
    /// Height in centimetres.
    pub height_cm: f64,
    /// Sex, as the Watson equations distinguish it.
    pub sex: Sex,
    /// Birth date. No age is ever stored: it is derived at the ingestion time of
    /// each drink (SPEC.md §10.0-K).
    #[schema(value_type = String, format = Date, example = "1994-05-12")]
    pub birth_date: NaiveDate,
}

/// A profile, as the API answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Profile {
    /// Identifier, a UUID v7 minted by the API.
    pub id: Uuid,
    /// Name shown in the profile picker.
    pub display_name: String,
    /// Unit the entry form pre-selects for a component.
    pub default_quantity_unit: QuantityUnit,
    /// Ingestion duration the entry form pre-fills, in seconds.
    pub default_ingestion_duration_seconds: i32,
    /// Absorption duration the entry form pre-fills, in seconds.
    pub default_absorption_duration_seconds: i32,
    /// Parameters in force now, or `null` when the profile owns no version.
    pub settings: Option<ProfileSettings>,
    /// Instant the profile was created.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub created_at: Timestamp,
    /// Instant the profile row was last written.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub updated_at: Timestamp,
}

/// The physiological half of a write, as it arrives.
///
/// Every member is optional so that a missing one is reported as a field error
/// naming it, rather than as an unreadable body: "a profile without a sex or a
/// birth date is rejected" is an acceptance criterion of issue #6, and a caller
/// is owed the name of what it left out.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct SettingsRequest {
    /// Instant the new values take effect. Defaults to now, never in the future
    /// (SPEC.md §10.0-J).
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub valid_from: Option<Timestamp>,
    /// Weight in kilograms.
    pub weight_kg: Option<f64>,
    /// Height in centimetres.
    pub height_cm: Option<f64>,
    /// Sex, as the Watson equations distinguish it.
    pub sex: Option<Sex>,
    /// Birth date.
    #[schema(value_type = Option<String>, format = Date, example = "1994-05-12")]
    pub birth_date: Option<NaiveDate>,
}

/// A profile creation or replacement, as it arrives.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ProfileRequest {
    /// Name shown in the profile picker.
    pub display_name: Option<String>,
    /// Unit the entry form pre-selects for a component.
    pub default_quantity_unit: Option<QuantityUnit>,
    /// Ingestion duration the entry form pre-fills, in seconds.
    pub default_ingestion_duration_seconds: Option<i32>,
    /// Absorption duration the entry form pre-fills, in seconds.
    pub default_absorption_duration_seconds: Option<i32>,
    /// The four physiological parameters, all mandatory (SPEC.md §10.0-D).
    pub settings: Option<SettingsRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labels a `CREATE TYPE ... AS ENUM (...)` of the migrations declares.
    fn enum_labels_in_the_migrations(type_name: &str) -> Vec<String> {
        let opening = format!("create type {type_name} as enum (");
        for migration in db::MIGRATOR.migrations.iter() {
            let lowered = migration.sql.to_lowercase();
            let Some(at) = lowered.find(&opening) else {
                continue;
            };
            let after = &migration.sql[at + opening.len()..];
            let body = after
                .split_once(')')
                .unwrap_or_else(|| panic!("unterminated `{opening}` in the migrations"))
                .0;
            return body
                .split(',')
                .map(|label| label.trim().trim_matches('\'').to_owned())
                .collect();
        }
        panic!("the migrations declare no `{type_name}` enum");
    }

    #[test]
    fn the_sql_labels_of_sex_are_the_ones_the_migrations_declare() {
        // Neither side may drift alone: `as_sql` is what every query binds, and
        // the migration is what PostgreSQL will accept. A label renamed on one
        // side only turns every write into a 500, which no other test would see.
        let declared = enum_labels_in_the_migrations("sex");
        let used: Vec<String> = Sex::ALL.iter().map(|v| v.as_sql().to_owned()).collect();
        assert_eq!(used, declared);
    }

    #[test]
    fn the_sql_labels_of_quantity_unit_are_the_ones_the_migrations_declare() {
        let declared = enum_labels_in_the_migrations("quantity_unit");
        let used: Vec<String> = QuantityUnit::ALL
            .iter()
            .map(|v| v.as_sql().to_owned())
            .collect();
        assert_eq!(used, declared);
    }

    #[test]
    fn an_enum_travels_on_the_wire_under_its_sql_label() {
        // One spelling for the column and for the payload, so that a client and
        // a `psql` session name the same value the same way.
        for value in Sex::ALL {
            let rendered = serde_json::to_value(value).expect("must serialise");
            assert_eq!(rendered, value.as_sql());
            assert_eq!(Sex::from_sql(value.as_sql()), Some(value));
        }
        for value in QuantityUnit::ALL {
            let rendered = serde_json::to_value(value).expect("must serialise");
            assert_eq!(rendered, value.as_sql());
            assert_eq!(QuantityUnit::from_sql(value.as_sql()), Some(value));
        }
    }

    #[test]
    fn an_unknown_label_is_not_read_as_a_variant() {
        assert_eq!(Sex::from_sql("other"), None);
        assert_eq!(QuantityUnit::from_sql("ml"), None);
    }

    fn a_profile(settings: Option<ProfileSettings>) -> Profile {
        Profile {
            id: crate::id::new_id(),
            display_name: "Alice".to_owned(),
            default_quantity_unit: QuantityUnit::Cl,
            default_ingestion_duration_seconds: 1200,
            default_absorption_duration_seconds: 1800,
            settings,
            created_at: crate::timestamp::now(),
            updated_at: crate::timestamp::now(),
        }
    }

    #[test]
    fn a_profile_without_settings_renders_them_as_null() {
        // The shape the empty result of `profile_settings_at()` reaches a client
        // as. A profile that lost its only version under the concurrency hole of
        // SPEC.md §10.0-L class (d) is reported, not hidden behind zeroes.
        let rendered = serde_json::to_value(a_profile(None)).expect("must serialise");
        assert!(rendered["settings"].is_null(), "{rendered}");
    }

    #[test]
    fn the_physiological_parameters_are_nested_and_the_preferences_are_not() {
        // Guards the shape that makes the versioned half visible: the four
        // parameters of SPEC.md §10.0-D under `settings`, the input preferences
        // at the top level. Flattening `settings` would make the payload claim
        // the profile carries them, which the arbitration of 2026-08-19 refuses.
        let rendered = serde_json::to_value(a_profile(Some(ProfileSettings {
            valid_from: crate::timestamp::now(),
            weight_kg: 62.0,
            height_cm: 168.0,
            sex: Sex::Female,
            birth_date: NaiveDate::from_ymd_opt(1994, 5, 12).expect("a real date"),
        })))
        .expect("must serialise");

        for versioned in ["weight_kg", "height_cm", "sex", "birth_date"] {
            assert!(
                rendered[versioned].is_null(),
                "`{versioned}` may not sit on the profile itself: {rendered}"
            );
            assert!(
                !rendered["settings"][versioned].is_null(),
                "`{versioned}` is missing from `settings`: {rendered}"
            );
        }
        for preference in [
            "display_name",
            "default_quantity_unit",
            "default_ingestion_duration_seconds",
            "default_absorption_duration_seconds",
        ] {
            assert!(
                !rendered[preference].is_null() && rendered["settings"][preference].is_null(),
                "`{preference}` is not versioned and belongs to the profile: {rendered}"
            );
        }
        assert_eq!(rendered["settings"]["birth_date"], "1994-05-12");
        assert!(
            rendered["age"].is_null()
                && rendered["settings"]["age"].is_null()
                && rendered["beta"].is_null()
                && rendered["settings"]["beta"].is_null(),
            "no age and no beta may reach the payload (SPEC.md §10.0-H, §10.0-K): {rendered}"
        );
    }
}
