//! Payload types of the event endpoints.
//!
//! An event travels with its participants, so that the view of an event
//! (SPEC.md §5.2) lists them without a request per profile. The participants
//! are managed under their own path, never by rewriting the event: a creation
//! may name the first ones, a replacement may not name any.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::timestamp::Timestamp;

/// A profile taking part in an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Participant {
    /// Identifier of the profile.
    pub profile_id: Uuid,
    /// Name of the profile, as the profile picker shows it.
    pub display_name: String,
    /// Instant the profile was added to the event.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub joined_at: Timestamp,
}

/// An event, as the API answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Event {
    /// Identifier, a UUID v7 minted by the API.
    pub id: Uuid,
    /// Name of the event.
    pub name: String,
    /// Instant the event starts.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:00:00Z")]
    pub starts_at: Timestamp,
    /// Instant the event ends, `null` while it is open-ended.
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-08-21T03:00:00Z")]
    pub ends_at: Option<Timestamp>,
    /// Profiles taking part, in the order they were added; profiles added
    /// together are ordered by identifier.
    pub participants: Vec<Participant>,
    /// Instant the event was created.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub created_at: Timestamp,
    /// Instant the event row was last written.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub updated_at: Timestamp,
}

/// An event creation or replacement, as it arrives on `POST /events` or
/// `PUT /events/{id}`.
///
/// Every member is optional so that a missing one is reported as a field
/// error naming it. A replacement rewrites the three members of the event
/// itself — an absent `ends_at` makes the event open-ended — and refuses
/// `participant_ids`: the participants of an existing event are added and
/// removed one by one under `/events/{id}/participants/{profile_id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, ToSchema)]
pub struct EventRequest {
    /// Name of the event, mandatory.
    pub name: Option<String>,
    /// Instant the event starts, mandatory.
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-08-20T21:00:00Z")]
    pub starts_at: Option<Timestamp>,
    /// Instant the event ends, not before `starts_at`. Absent or `null`: the
    /// event is open-ended.
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-08-21T03:00:00Z")]
    pub ends_at: Option<Timestamp>,
    /// Profiles taking part from the start, creation only: on a creation a
    /// profile named twice is added once, and `null` names nobody; on a
    /// replacement the member is refused whenever present, `null` included.
    #[serde(default, deserialize_with = "crate::drink::model::present")]
    #[schema(value_type = Option<Vec<Uuid>>)]
    pub participant_ids: Option<Option<Vec<Uuid>>>,
}
