//! Drinks of a profile: recorded, listed, modified, deleted (SPEC.md §4, §5.3,
//! §5.5).
//!
//! The module is split like [`crate::profile`]: [`model`] holds the payload
//! types, [`validation`] reads a write, [`store`] holds every statement, and
//! this file holds the handlers. What is worth knowing before reading them:
//!
//! - a drink always belongs to the history of its profile; entered from an
//!   event, it is the same row with `event_id` set, so it is in the profile's
//!   history all the same (SPEC.md §4, §5.5);
//! - a creation falls back on the instant of the request for `ingested_at`,
//!   and on the input preferences of the profile for the two durations
//!   (SPEC.md §5.3);
//! - a modification merges onto the stored drink and the result is held to
//!   the rules of a creation; a new composition replaces the whole old one;
//! - the history lists every drink of the profile, latest ingestion first,
//!   each with its whole composition, so that mode 3 of SPEC.md §5.3 can
//!   duplicate one without another request ([`history`]);
//! - nothing is cached: no alcohol series is stored anywhere, so a write has
//!   no cache to invalidate, and any later computation reads the drinks as
//!   they are now.

pub mod history;
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
use crate::timestamp;
use crate::{AppState, ProblemDetails};

use history::HistoryQuery;

pub use model::{Component, ComponentRequest, Drink, DrinkPatch, DrinkRequest};

/// Where the drinks of a profile are recorded and listed, in the path syntax
/// of axum 0.8.
pub const PROFILE_COLLECTION_PATH: &str = "/profiles/{id}/drinks";

/// Where a single drink is served.
pub const ITEM_PATH: &str = "/drinks/{id}";

/// Prefix of [`ITEM_PATH`], from which the `Location` of a creation is built.
const ITEM_PREFIX: &str = "/drinks/";

fn no_such_drink(id: Uuid) -> ApiError {
    ApiError::not_found(format!("no drink with identifier {id}"))
}

/// Reads a drink that was just written, which has to be there.
async fn written(state: &AppState, id: Uuid) -> Result<Drink, ApiError> {
    store::read(&state.pool, id)
        .await?
        .ok_or_else(|| no_such_drink(id))
}

/// Records a drink in the history of a profile, optionally from an event.
#[utoipa::path(
    post,
    path = PROFILE_COLLECTION_PATH,
    tag = "drinks",
    params(("id" = Uuid, Path, description = "Identifier of the profile")),
    request_body = DrinkRequest,
    responses(
        (status = 201, description = "The drink as recorded", body = Drink,
         headers(("location" = String, description = "Path of the new drink"))),
        (status = 400, description = "The payload is unreadable or invalid, or names an event the profile does not take part in", body = ProblemDetails),
        (status = 404, description = "No profile carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    JsonBody(request): JsonBody<DrinkRequest>,
) -> Result<Response, ApiError> {
    let now = timestamp::now();
    let id = new_id();
    let outcome = store::create(&state.pool, id, profile_id, |defaults| {
        request.validate(defaults, now)
    })
    .await?;
    if outcome == store::Written::NotFound {
        return Err(ApiError::not_found(format!(
            "no profile with identifier {profile_id}"
        )));
    }

    let created = written(&state, id).await?;
    let location = format!("{ITEM_PREFIX}{id}");
    let mut response = (StatusCode::CREATED, Json(created)).into_response();
    let header_value = HeaderValue::from_str(&location)
        .map_err(|error| ApiError::internal(format!("unusable Location `{location}`: {error}")))?;
    response
        .headers_mut()
        .insert(header::LOCATION, header_value);
    Ok(response)
}

/// Lists the history of a profile: every drink, event or not, latest
/// ingestion first, each with its components.
#[utoipa::path(
    get,
    path = PROFILE_COLLECTION_PATH,
    tag = "drinks",
    params(
        ("id" = Uuid, Path, description = "Identifier of the profile"),
        HistoryQuery,
        PageQuery,
    ),
    responses(
        (status = 200, description = "One page of the history, latest ingestion first", body = Page<Drink>),
        (status = 400, description = "Unusable identifier, `from`, `before`, `event_id`, `limit` or `cursor`", body = ProblemDetails),
        (status = 404, description = "No profile carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Query(filter): Query<HistoryQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Drink>>, ApiError> {
    // Both halves are read before either is reported, so that one answer
    // lists every member a client got wrong.
    let (filter, request) = match (filter.into_filter(), page.into_parts()) {
        (Ok(filter), Ok(request)) => (filter, request),
        (filter, request) => {
            let mut errors = filter.err().unwrap_or_default();
            errors.extend(request.err().unwrap_or_default());
            return Err(ApiError::validation(errors));
        }
    };
    store::history(&state.pool, profile_id, &filter, &request)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("no profile with identifier {profile_id}")))
}

/// Reads one drink with its components.
#[utoipa::path(
    get,
    path = ITEM_PATH,
    tag = "drinks",
    params(("id" = Uuid, Path, description = "Identifier of the drink")),
    responses(
        (status = 200, description = "The drink", body = Drink),
        (status = 400, description = "The identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No drink carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Drink>, ApiError> {
    written(&state, id).await.map(Json)
}

/// Modifies a drink: composition, times, durations, name, feeling rating or
/// event. Absent members keep their stored value.
#[utoipa::path(
    patch,
    path = ITEM_PATH,
    tag = "drinks",
    params(("id" = Uuid, Path, description = "Identifier of the drink")),
    request_body = DrinkPatch,
    responses(
        (status = 200, description = "The drink as modified", body = Drink),
        (status = 400, description = "The payload is unreadable, or the modified drink is invalid", body = ProblemDetails),
        (status = 404, description = "No drink carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    JsonBody(patch): JsonBody<DrinkPatch>,
) -> Result<Json<Drink>, ApiError> {
    let replace_components = matches!(patch.components, Some(Some(_)));
    let outcome = store::update(&state.pool, id, replace_components, |stored| {
        patch.merge_onto(stored)
    })
    .await?;
    if outcome == store::Written::NotFound {
        return Err(no_such_drink(id));
    }
    written(&state, id).await.map(Json)
}

/// Deletes a drink and its components.
#[utoipa::path(
    delete,
    path = ITEM_PATH,
    tag = "drinks",
    params(("id" = Uuid, Path, description = "Identifier of the drink")),
    responses(
        (status = 204, description = "The drink is deleted"),
        (status = 400, description = "The identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No drink carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    match store::delete(&state.pool, id).await? {
        store::Written::NotFound => Err(no_such_drink(id)),
        store::Written::Done => Ok(StatusCode::NO_CONTENT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_location_of_a_creation_is_the_item_path() {
        assert_eq!(ITEM_PATH, format!("{ITEM_PREFIX}{{id}}"));
        assert!(PROFILE_COLLECTION_PATH.starts_with(crate::profile::ITEM_PATH));
    }
}
