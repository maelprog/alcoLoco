//! `/events` — the events of SPEC.md §5.2 and the profiles taking part.
//!
//! The module is split like [`crate::drink`]: [`model`] holds the payload
//! types, [`validation`] reads a write, [`store`] holds every statement, and
//! this file holds the handlers. What is worth knowing before reading them:
//!
//! - an event has a name, a start and an end; a `null` end leaves it
//!   open-ended, and an end is never before the start;
//! - profiles take part directly, without an invitation (SPEC.md §5.2, V1):
//!   a creation may name the first ones, and
//!   `/events/{id}/participants/{profile_id}` adds (`PUT`) or removes
//!   (`DELETE`) one at a time afterwards; a profile may take part in any
//!   number of events;
//! - removing a participant never deletes a drink: a drink belongs to the
//!   history of its profile whatever the event (SPEC.md §4), and the drinks
//!   it recorded from the event are kept there, detached from the event;
//! - consumption is always tracked in V1: there is no per-event toggle
//!   (SPEC.md §5.2, #30).

pub mod model;
pub mod store;
pub mod validation;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::{Json as JsonBody, Path, Query};
use crate::id::new_id;
use crate::pagination::{Page, PageQuery};
use crate::{AppState, ProblemDetails};

pub use model::{Event, EventRequest, Participant};

/// Where the collection is served.
pub const COLLECTION_PATH: &str = "/events";

/// Where a single event is served, in the path syntax of axum 0.8.
pub const ITEM_PATH: &str = "/events/{id}";

/// Where one participant of an event is added and removed.
pub const PARTICIPANT_PATH: &str = "/events/{id}/participants/{profile_id}";

fn no_such_event(id: Uuid) -> ApiError {
    ApiError::not_found(format!("no event with identifier {id}"))
}

fn no_such_profile(id: Uuid) -> ApiError {
    ApiError::not_found(format!("no profile with identifier {id}"))
}

/// Reads an event that was just written, which has to be there.
async fn written(state: &AppState, id: Uuid) -> Result<Event, ApiError> {
    store::read(&state.pool, id)
        .await?
        .ok_or_else(|| no_such_event(id))
}

/// Lists the events, in creation order, one page at a time.
#[utoipa::path(
    get,
    path = COLLECTION_PATH,
    tag = "events",
    params(PageQuery),
    responses(
        (status = 200, description = "One page of events, in creation order, each with its participants", body = Page<Event>),
        (status = 400, description = "Unusable `limit` or `cursor`", body = ProblemDetails),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Event>>, ApiError> {
    let request = page.into_request()?;
    Ok(Json(store::list(&state.pool, &request).await?))
}

/// Reads one event with its participants.
#[utoipa::path(
    get,
    path = ITEM_PATH,
    tag = "events",
    params(("id" = Uuid, Path, description = "Identifier of the event")),
    responses(
        (status = 200, description = "The event", body = Event),
        (status = 400, description = "The identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No event carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Event>, ApiError> {
    written(&state, id).await.map(Json)
}

/// Creates an event, with the profiles taking part from the start.
#[utoipa::path(
    post,
    path = COLLECTION_PATH,
    tag = "events",
    request_body = EventRequest,
    responses(
        (status = 201, description = "The event as created", body = Event,
         headers(("location" = String, description = "Path of the new event"))),
        (status = 400, description = "The payload is unreadable or invalid, or names a participant that is no profile", body = ProblemDetails),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<EventRequest>,
) -> Result<Response, ApiError> {
    let creation = request.validate_creation()?;
    let id = new_id();
    store::create(&state.pool, id, &creation).await?;

    let created = written(&state, id).await?;
    let location = format!("{COLLECTION_PATH}/{id}");
    let mut response = (StatusCode::CREATED, Json(created)).into_response();
    let header_value = HeaderValue::from_str(&location)
        .map_err(|error| ApiError::internal(format!("unusable Location `{location}`: {error}")))?;
    response
        .headers_mut()
        .insert(header::LOCATION, header_value);
    Ok(response)
}

/// Replaces the name, start and end of an event. Its participants are left
/// as they are; an absent `ends_at` makes the event open-ended.
#[utoipa::path(
    put,
    path = ITEM_PATH,
    tag = "events",
    params(("id" = Uuid, Path, description = "Identifier of the event")),
    request_body = EventRequest,
    responses(
        (status = 200, description = "The event as replaced", body = Event),
        (status = 400, description = "The payload is unreadable or invalid, or names participants", body = ProblemDetails),
        (status = 404, description = "No event carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<EventRequest>,
) -> Result<Json<Event>, ApiError> {
    let event = request.validate_replacement()?;
    match store::update(&state.pool, id, &event).await? {
        store::Written::Done => written(&state, id).await.map(Json),
        _ => Err(no_such_event(id)),
    }
}

/// Adds a profile to an event. Adding a profile already taking part changes
/// nothing and answers the same.
#[utoipa::path(
    put,
    path = PARTICIPANT_PATH,
    tag = "events",
    params(
        ("id" = Uuid, Path, description = "Identifier of the event"),
        ("profile_id" = Uuid, Path, description = "Identifier of the profile"),
    ),
    responses(
        (status = 204, description = "The profile takes part in the event"),
        (status = 400, description = "An identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No event, or no profile, carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn add_participant(
    State(state): State<AppState>,
    Path((id, profile_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    match store::add_participant(&state.pool, id, profile_id).await? {
        store::Written::Done => Ok(StatusCode::NO_CONTENT),
        store::Written::NoProfile => Err(no_such_profile(profile_id)),
        _ => Err(no_such_event(id)),
    }
}

/// Removes a profile from an event. The drinks it recorded from the event
/// stay in its history, detached from the event.
#[utoipa::path(
    delete,
    path = PARTICIPANT_PATH,
    tag = "events",
    params(
        ("id" = Uuid, Path, description = "Identifier of the event"),
        ("profile_id" = Uuid, Path, description = "Identifier of the profile"),
    ),
    responses(
        (status = 204, description = "The profile no longer takes part in the event"),
        (status = 400, description = "An identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No event carries that identifier, or the profile does not take part in it", body = ProblemDetails),
    ),
)]
pub async fn remove_participant(
    State(state): State<AppState>,
    Path((id, profile_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    match store::remove_participant(&state.pool, id, profile_id).await? {
        store::Written::Done => Ok(StatusCode::NO_CONTENT),
        store::Written::NotParticipating => Err(ApiError::not_found(format!(
            "profile {profile_id} does not take part in event {id}"
        ))),
        _ => Err(no_such_event(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_paths_agree_on_the_collection_they_serve() {
        // The `Location` of a creation is built from `COLLECTION_PATH`; it has
        // to land on the path the router serves an event on.
        assert_eq!(ITEM_PATH, format!("{COLLECTION_PATH}/{{id}}"));
        assert_eq!(
            PARTICIPANT_PATH,
            format!("{ITEM_PATH}/participants/{{profile_id}}")
        );
    }
}
