//! Payload types of the drink endpoints.
//!
//! A drink travels with its components, in the order they were added, and
//! with the members of the `drink` table spelled as its columns. A component
//! quantity is in **centilitres** when the drink is in `cl` — the unit the
//! user types — and a **percentage of `volume_total_ml`** when it is in
//! `percent` (SPEC.md §5.3, §10.0-A). It is stored exactly as it travels
//! (migration `20261022090000`), so what a client writes is what it reads
//! back, to the bit; the conversion to millilitres, the unit of the domain,
//! happens only where the rules of a drink are applied
//! ([`super::validation`]). `volume_total_ml` stays in millilitres.

use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::profile::QuantityUnit;
use crate::timestamp::Timestamp;

/// One component of a drink, as the API answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Component {
    /// Name of the component, e.g. `Vodka`.
    pub name: String,
    /// Alcohol by volume, in percent, between 0 and 100.
    pub abv_percent: f64,
    /// Centilitres when the drink is in `cl`, percent of `volume_total_ml`
    /// when it is in `percent`.
    pub quantity: f64,
}

/// A drink, as the API answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Drink {
    /// Identifier, a UUID v7 minted by the API.
    pub id: Uuid,
    /// Profile whose history the drink belongs to — always, event or not
    /// (SPEC.md §4).
    pub profile_id: Uuid,
    /// Event the drink was entered from, `null` when it was not.
    pub event_id: Option<Uuid>,
    /// Name of the drink.
    pub name: String,
    /// Instant the drink started being drunk.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub ingested_at: Timestamp,
    /// Ingestion duration, in seconds.
    pub ingestion_duration_seconds: i32,
    /// Absorption duration, in seconds.
    pub absorption_duration_seconds: i32,
    /// Unit shared by the quantities of every component.
    pub quantity_unit: QuantityUnit,
    /// Total volume in millilitres as entered: set in `percent`, `null` in
    /// `cl`, where it is the sum of the components.
    pub volume_total_ml: Option<f64>,
    /// Feeling rating, 1 to 10 (SPEC.md §5.9), `null` when none was given.
    pub feeling_rating: Option<i16>,
    /// Components, in the order they were added.
    pub components: Vec<Component>,
    /// Instant the drink was recorded.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub created_at: Timestamp,
    /// Instant the drink row was last written.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub updated_at: Timestamp,
}

/// A component as it arrives. Every member is optional so that a missing one
/// is reported as a field error naming it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, ToSchema)]
pub struct ComponentRequest {
    /// Name of the component.
    pub name: Option<String>,
    /// Alcohol by volume, in percent.
    pub abv_percent: Option<f64>,
    /// Centilitres in `cl`, percent of `volume_total_ml` in `percent`.
    pub quantity: Option<f64>,
}

impl From<Component> for ComponentRequest {
    fn from(component: Component) -> Self {
        Self {
            name: Some(component.name),
            abv_percent: Some(component.abv_percent),
            quantity: Some(component.quantity),
        }
    }
}

/// A drink creation, as it arrives on `POST /profiles/{id}/drinks`.
///
/// `ingested_at` defaults to the instant of the request, and the two durations
/// to the input preferences of the profile (SPEC.md §5.3). The unit has no
/// default: it decides how every quantity of the payload is read.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, ToSchema)]
pub struct DrinkRequest {
    /// Event the drink is entered from. The profile must take part in it.
    pub event_id: Option<Uuid>,
    /// Name of the drink, mandatory.
    pub name: Option<String>,
    /// Instant the drink started being drunk. Defaults to now.
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub ingested_at: Option<Timestamp>,
    /// Ingestion duration, in seconds. Defaults to the profile preference.
    pub ingestion_duration_seconds: Option<i64>,
    /// Absorption duration, in seconds. Defaults to the profile preference.
    pub absorption_duration_seconds: Option<i64>,
    /// Unit of every component quantity, mandatory.
    pub quantity_unit: Option<QuantityUnit>,
    /// Total volume in millilitres: mandatory in `percent`, refused in `cl`.
    pub volume_total_ml: Option<f64>,
    /// Feeling rating, 1 to 10, optional.
    pub feeling_rating: Option<i64>,
    /// One or more components, in the order they were added.
    pub components: Option<Vec<ComponentRequest>>,
}

/// A drink modification, as it arrives on `PATCH /drinks/{id}`.
///
/// An absent member keeps the stored value. Every member tells an absent
/// member from an explicit `null`: on the three nullable members —
/// `event_id`, `volume_total_ml`, `feeling_rating` — `null` clears the stored
/// value; on any other it is refused, field by field, since those members
/// cannot be cleared. `components`, when present, replaces the whole
/// composition; it has to be present when `quantity_unit` changes, since the
/// stored quantities mean something else in the other unit.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, ToSchema)]
pub struct DrinkPatch {
    /// Event the drink belongs to; `null` detaches it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>)]
    pub event_id: Option<Option<Uuid>>,
    /// Name of the drink. Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = String)]
    pub name: Option<Option<String>>,
    /// Instant the drink started being drunk. Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub ingested_at: Option<Option<Timestamp>>,
    /// Ingestion duration, in seconds. Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = i64)]
    pub ingestion_duration_seconds: Option<Option<i64>>,
    /// Absorption duration, in seconds. Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = i64)]
    pub absorption_duration_seconds: Option<Option<i64>>,
    /// Unit of every component quantity. Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = QuantityUnit)]
    pub quantity_unit: Option<Option<QuantityUnit>>,
    /// Total volume in millilitres; `null` clears it (a drink moving to `cl`).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<f64>)]
    pub volume_total_ml: Option<Option<f64>>,
    /// Feeling rating, 1 to 10; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>)]
    pub feeling_rating: Option<Option<i64>>,
    /// The whole new composition, in the order the components were added.
    /// Not nullable.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Vec<ComponentRequest>)]
    pub components: Option<Option<Vec<ComponentRequest>>>,
}

/// Reads a member that is present, `null` included, as `Some`: together with
/// `#[serde(default)]`, an absent member stays `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_tells_an_absent_member_from_an_explicit_null() {
        let absent: DrinkPatch = serde_json::from_str("{}").expect("must parse");
        assert_eq!(absent.feeling_rating, None);
        assert_eq!(absent.volume_total_ml, None);
        assert_eq!(absent.event_id, None);

        let cleared: DrinkPatch = serde_json::from_str(
            r#"{"feeling_rating":null,"volume_total_ml":null,"event_id":null}"#,
        )
        .expect("must parse");
        assert_eq!(cleared.feeling_rating, Some(None));
        assert_eq!(cleared.volume_total_ml, Some(None));
        assert_eq!(cleared.event_id, Some(None));

        let set: DrinkPatch =
            serde_json::from_str(r#"{"feeling_rating":4,"volume_total_ml":250.0}"#)
                .expect("must parse");
        assert_eq!(set.feeling_rating, Some(Some(4)));
        assert_eq!(set.volume_total_ml, Some(Some(250.0)));
    }
}
