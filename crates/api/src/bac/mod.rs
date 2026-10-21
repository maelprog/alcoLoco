//! Blood alcohol curves: of a profile, and of every participant of an event
//! (SPEC.md §5.6, §5.7).
//!
//! The module is split like [`crate::drink`]: [`model`] holds the payload
//! types, [`query`] reads the window and the step, [`store`] holds every
//! statement, [`series`] turns the stored drinks into values, and this file
//! holds the handlers. What is worth knowing before reading them:
//!
//! - the values are those of `domain::curve`, under the default absorption
//!   profile ([`domain::absorption::Trapezoid`], pending #37) and the constant
//!   elimination rate of the domain (SPEC.md §10.0-H); nothing is cached, so
//!   a drink written a moment ago is in the next answer;
//! - every drink of a profile counts, whatever the event it was entered from:
//!   the curve is the body's; an event only chooses the profiles and the
//!   window;
//! - each drink is computed with the settings version in force at its
//!   ingestion time and the age then (SPEC.md §5.6, §10.0-K);
//! - every value is exact at its instant, so the step only thins out the
//!   samples (SPEC.md §10.0-I); it is bounded to `[1 min, 1 h]` and a series
//!   holds at most [`query::MAX_SAMPLES`] samples;
//! - instants are counted in whole seconds: a drink starts at the second of
//!   its ingestion, rounded down, and the current value is that of the second
//!   of the request.

pub mod model;
pub mod query;
pub mod series;
pub mod store;

use axum::Json;
use axum::extract::State;
use chrono::TimeDelta;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::{Path, Query};
use crate::timestamp::{self, Timestamp};
use crate::{AppState, ProblemDetails};

pub use model::{EventBac, ParticipantBac, ProfileBac};
use query::{EventBacQuery, Grid, ProfileBacQuery, instant, second_at_or_before};

/// Where the curve of a profile is served, in the path syntax of axum 0.8.
pub const PROFILE_PATH: &str = "/profiles/{id}/bac";

/// Where the curves of an event are served.
pub const EVENT_PATH: &str = "/events/{id}/bac";

/// The instant `seconds` after the epoch, which a grid built from real
/// instants always has.
fn at(seconds: i64) -> Result<Timestamp, ApiError> {
    instant(seconds).ok_or_else(|| ApiError::internal(format!("no instant at {seconds} s")))
}

/// The exclusive bound on the ingestion time of the drinks an answer reads:
/// a drink starting after both the window and the request changes no value
/// of it.
fn drinks_before(grid: Grid, now: Timestamp) -> Result<Timestamp, ApiError> {
    at(grid.to.max(second_at_or_before(now)))?
        .checked_add_signed(TimeDelta::seconds(1))
        .ok_or_else(|| ApiError::internal("no instant after the window"))
}

/// The curve of a profile over a window, and its current value.
#[utoipa::path(
    get,
    path = PROFILE_PATH,
    tag = "curves",
    params(
        ("id" = Uuid, Path, description = "Identifier of the profile"),
        ProfileBacQuery,
    ),
    responses(
        (status = 200, description = "The curve over the window, and the value now", body = ProfileBac),
        (status = 400, description = "Unusable identifier, `from`, `to` or `step`, or a window holding too many samples", body = ProblemDetails),
        (status = 404, description = "No profile carries that identifier", body = ProblemDetails),
        (status = 409, description = "The settings in force at a drink give no positive body water", body = ProblemDetails),
    ),
)]
pub async fn profile(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Query(query): Query<ProfileBacQuery>,
) -> Result<Json<ProfileBac>, ApiError> {
    let grid = query.into_grid().map_err(ApiError::validation)?;
    let now = timestamp::now();
    let drinks = store::profile_drinks(&state.pool, profile_id, drinks_before(grid, now)?)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("no profile with identifier {profile_id}")))?;
    let intakes = drinks
        .iter()
        .map(|drink| series::intake(profile_id, drink))
        .collect::<Result<Vec<_>, _>>()?;
    let values = series::values(&intakes, grid, now)?;
    Ok(Json(ProfileBac {
        profile_id,
        computed_at: at(second_at_or_before(now))?,
        current_bac_g_per_l: values.current,
        from: at(grid.from)?,
        to: at(grid.to)?,
        step_seconds: grid.step,
        times: grid.times(),
        bac_g_per_l: values.series,
    }))
}

/// The curves of every participant of an event, on one grid spanning the
/// event and the three hours after it, and their current values.
#[utoipa::path(
    get,
    path = EVENT_PATH,
    tag = "curves",
    params(
        ("id" = Uuid, Path, description = "Identifier of the event"),
        EventBacQuery,
    ),
    responses(
        (status = 200, description = "One curve per participant on a shared grid, and the values now", body = EventBac),
        (status = 400, description = "Unusable identifier or `step`, or a step too fine for the window of the event", body = ProblemDetails),
        (status = 404, description = "No event carries that identifier", body = ProblemDetails),
        (status = 409, description = "The settings in force at a drink give no positive body water", body = ProblemDetails),
    ),
)]
pub async fn event(
    State(state): State<AppState>,
    Path(event_id): Path<Uuid>,
    Query(query): Query<EventBacQuery>,
) -> Result<Json<EventBac>, ApiError> {
    let now = timestamp::now();
    let event = store::event_drinks(
        &state.pool,
        event_id,
        |starts_at, ends_at| {
            query
                .into_grid(starts_at, ends_at, now)
                .map_err(ApiError::validation)
        },
        |grid| drinks_before(grid, now),
    )
    .await?
    .ok_or_else(|| ApiError::not_found(format!("no event with identifier {event_id}")))?;

    let grid = event.grid;
    let participants = event
        .participants
        .into_iter()
        .map(|participant| {
            let intakes = participant
                .drinks
                .iter()
                .map(|drink| series::intake(participant.profile_id, drink))
                .collect::<Result<Vec<_>, _>>()?;
            let values = series::values(&intakes, grid, now)?;
            Ok(ParticipantBac {
                profile_id: participant.profile_id,
                display_name: participant.display_name,
                current_bac_g_per_l: values.current,
                bac_g_per_l: values.series,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(EventBac {
        event_id,
        computed_at: at(second_at_or_before(now))?,
        from: at(grid.from)?,
        to: at(grid.to)?,
        step_seconds: grid.step,
        times: grid.times(),
        participants,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curves_hang_under_the_resources_they_are_of() {
        assert_eq!(PROFILE_PATH, format!("{}/bac", crate::profile::ITEM_PATH));
        assert_eq!(EVENT_PATH, format!("{}/bac", crate::event::ITEM_PATH));
    }

    #[test]
    fn the_drinks_read_reach_the_later_of_the_window_and_now() {
        let now: Timestamp = "2026-08-20T22:00:00.4Z".parse().expect("a real instant");
        let window = Grid {
            from: 0,
            to: now.timestamp() + 600,
            step: 60,
        };
        assert_eq!(
            drinks_before(window, now).expect("an instant").timestamp(),
            now.timestamp() + 601
        );
        let past = Grid {
            from: 0,
            to: 60,
            step: 60,
        };
        // A drink started within the second of the request counts now.
        assert_eq!(
            drinks_before(past, now).expect("an instant").timestamp(),
            now.timestamp() + 1
        );
    }
}
