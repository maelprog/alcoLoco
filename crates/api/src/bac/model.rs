//! Payload types of the curve endpoints.
//!
//! A series travels as two parallel arrays, `times` and `bac_g_per_l`, so that
//! the front plots it as it comes. An event answers `times` once, shared by
//! the series of every participant: the grid is common by construction, not
//! by coincidence.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::timestamp::Timestamp;

/// The curve of one profile over the window asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProfileBac {
    /// Identifier of the profile.
    pub profile_id: Uuid,
    /// Instant of the current value: the instant of the request, truncated
    /// to the minute.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T23:04:00Z")]
    pub computed_at: Timestamp,
    /// Blood alcohol concentration at `computed_at`, in grams per litre.
    pub current_bac_g_per_l: f64,
    /// First instant of the window, `from` truncated to the minute.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T20:00:00Z")]
    pub from: Timestamp,
    /// Last instant of the window, `to` truncated to the minute; sampled when
    /// it falls on the step.
    #[schema(value_type = String, format = DateTime, example = "2026-08-21T04:00:00Z")]
    pub to: Timestamp,
    /// Seconds between two samples actually used: the step asked for, or,
    /// when the answer would exceed its cap of 10 081 values, the smallest
    /// multiple of 60 s that fits — possibly coarser than an hour.
    pub step_seconds: i64,
    /// Instants sampled, from `from` every `step_seconds`.
    #[schema(value_type = Vec<String>, example = json!(["2026-08-20T20:00:00Z", "2026-08-20T20:01:00Z"]))]
    pub times: Vec<Timestamp>,
    /// Blood alcohol concentration at each of `times`, in grams per litre,
    /// never negative.
    pub bac_g_per_l: Vec<f64>,
}

/// The feeling rating suggested to a profile at an instant (SPEC.md §5.3,
/// §5.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FeelingSuggestion {
    /// Instant of the suggestion: `at`, or the instant of the request,
    /// truncated to the minute.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T23:04:00Z")]
    pub at: Timestamp,
    /// Blood alcohol concentration at `at`, in grams per litre: the value of
    /// the curve of the profile at that instant, unrounded, from the drinks
    /// stored — the drink being entered is not one of them yet.
    pub bac_g_per_l: f64,
    /// Feeling rating suggested, from 1 to 6: the scale of SPEC.md §5.9
    /// applied to `bac_g_per_l` rounded to two decimals, each band including
    /// its lower bound. Levels 7 to 10 are never suggested.
    #[schema(minimum = 1, maximum = 6, example = 2)]
    pub rating: u8,
}

/// The curve of one participant of an event, on the grid of the event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ParticipantBac {
    /// Identifier of the profile.
    pub profile_id: Uuid,
    /// Name of the profile.
    pub display_name: String,
    /// Blood alcohol concentration at the `computed_at` of the event answer,
    /// in grams per litre.
    pub current_bac_g_per_l: f64,
    /// Blood alcohol concentration at each of the `times` of the event answer,
    /// in grams per litre, never negative.
    pub bac_g_per_l: Vec<f64>,
}

/// The curves of every participant of an event, on one shared grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EventBac {
    /// Identifier of the event.
    pub event_id: Uuid,
    /// Instant of the current values: the instant of the request, truncated
    /// to the minute.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T23:04:00Z")]
    pub computed_at: Timestamp,
    /// First instant of the window: the start of the event, truncated to the
    /// minute.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T20:00:00Z")]
    pub from: Timestamp,
    /// Last instant of the window: three hours after the end of the event,
    /// or after `computed_at` while the event is open-ended (SPEC.md §10.3).
    /// An event yet to start is sampled all the same, as a projected curve,
    /// its participants having possibly drunk before it, whether its end is
    /// fixed or open; `times` is empty only when this instant is
    /// earlier than `from`, i.e. for an open-ended event starting more than
    /// three hours after the request.
    #[schema(value_type = String, format = DateTime, example = "2026-08-21T05:00:00Z")]
    pub to: Timestamp,
    /// Seconds between two samples actually used: the step asked for, or,
    /// when the answer would exceed its cap of 10 081 values, the smallest
    /// multiple of 60 s that fits — possibly coarser than an hour.
    pub step_seconds: i64,
    /// Instants sampled, from `from` every `step_seconds`, shared by every
    /// participant.
    #[schema(value_type = Vec<String>, example = json!(["2026-08-20T20:00:00Z", "2026-08-20T20:01:00Z"]))]
    pub times: Vec<Timestamp>,
    /// One curve per participant, in the order they were added.
    pub participants: Vec<ParticipantBac>,
}
