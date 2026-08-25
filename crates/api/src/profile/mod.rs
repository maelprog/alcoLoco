//! `/profiles` — the profiles of SPEC.md §5.1, without authentication in V1.
//!
//! The module is split three ways: [`model`] holds the payload types, [`store`]
//! holds every statement, and this file holds the four handlers. What is worth
//! knowing before reading any of them:
//!
//! - a profile carries its identity and its **input preferences**; the four
//!   physiological parameters live in `profile_settings_version` and reach the
//!   payload under `settings`, read through `profile_settings_at(id, now())`
//!   (SPEC.md §4, §5.1, §10.0-L);
//! - a write that changes those four parameters posts a settings version at the
//!   `valid_from` the caller chose, defaulting to now and never in the future;
//!   a write that changes only the preferences posts none (SPEC.md §10.0-J);
//! - the rest of the versioning policy — closing the previous version at `T`,
//!   replacing every version already later than `T` and reporting how many were
//!   replaced (SPEC.md §5.1) — is issue #7, and #43 blocks it.

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

pub use model::{Profile, ProfileRequest, ProfileSettings, QuantityUnit, SettingsRequest, Sex};
pub use validation::{ValidProfile, ValidSettings};

/// Where the collection is served.
pub const COLLECTION_PATH: &str = "/profiles";

/// Where a single profile is served, in the path syntax of axum 0.8.
pub const ITEM_PATH: &str = "/profiles/{id}";

/// Says a profile carries no identifier of that value, in the same words
/// whichever handler noticed.
fn no_such_profile(id: Uuid) -> ApiError {
    ApiError::not_found(format!("no profile with identifier {id}"))
}

/// Lists the profiles, oldest first, one page at a time.
///
/// No authentication: choosing a profile from this list is the entry point of
/// the application in V1 (SPEC.md §5.1).
#[utoipa::path(
    get,
    path = COLLECTION_PATH,
    tag = "profiles",
    params(PageQuery),
    responses(
        (status = 200, description = "One page of profiles, in creation order", body = Page<Profile>),
        (status = 400, description = "Unusable `limit` or `cursor`", body = ProblemDetails),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Profile>>, ApiError> {
    let request = page.into_request()?;
    Ok(Json(store::list(&state.pool, request).await?))
}

/// Reads one profile and the parameters in force for it now.
#[utoipa::path(
    get,
    path = ITEM_PATH,
    tag = "profiles",
    params(("id" = Uuid, Path, description = "Identifier of the profile")),
    responses(
        (status = 200, description = "The profile", body = Profile),
        (status = 400, description = "The identifier is not a UUID", body = ProblemDetails),
        (status = 404, description = "No profile carries that identifier", body = ProblemDetails),
    ),
)]
pub async fn read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Profile>, ApiError> {
    store::read(&state.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| no_such_profile(id))
}

/// Creates a profile together with its first settings version.
///
/// The four physiological parameters are mandatory: without a sex and a birth
/// date the computation of SPEC.md §6.2 cannot run, so a request missing either
/// is refused rather than stored (acceptance criterion of issue #6).
#[utoipa::path(
    post,
    path = COLLECTION_PATH,
    tag = "profiles",
    request_body = ProfileRequest,
    responses(
        (status = 201, description = "The profile as created", body = Profile,
         headers(("location" = String, description = "Path of the new profile"))),
        (status = 400, description = "The payload is unreadable or invalid", body = ProblemDetails),
        (status = 409, description = "A settings version already starts at that `valid_from`", body = ProblemDetails),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<ProfileRequest>,
) -> Result<Response, ApiError> {
    let profile = request.validate(timestamp::now())?;
    let id = new_id();
    store::create(&state.pool, id, &profile).await?;

    let created = store::read(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::internal(format!("profile {id} vanished after its creation")))?;

    let location = format!("{COLLECTION_PATH}/{id}");
    let mut response = (StatusCode::CREATED, Json(created)).into_response();
    let header_value = HeaderValue::from_str(&location)
        .map_err(|error| ApiError::internal(format!("unusable Location `{location}`: {error}")))?;
    response
        .headers_mut()
        .insert(header::LOCATION, header_value);
    Ok(response)
}

/// Replaces a profile.
///
/// A settings version is posted only when the four physiological parameters
/// differ from those in force at the requested `valid_from`. Rewriting the input
/// preferences alone posts none: they enter no computation (SPEC.md §10.0-J).
#[utoipa::path(
    put,
    path = ITEM_PATH,
    tag = "profiles",
    params(("id" = Uuid, Path, description = "Identifier of the profile")),
    request_body = ProfileRequest,
    responses(
        (status = 200, description = "The profile as replaced", body = Profile),
        (status = 400, description = "The payload is unreadable or invalid", body = ProblemDetails),
        (status = 404, description = "No profile carries that identifier", body = ProblemDetails),
        (status = 409, description = "A settings version already starts at that `valid_from`", body = ProblemDetails),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<ProfileRequest>,
) -> Result<Json<Profile>, ApiError> {
    let profile = request.validate(timestamp::now())?;

    if store::update(&state.pool, id, &profile).await? == store::Updated::NoSuchProfile {
        return Err(no_such_profile(id));
    }

    store::read(&state.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| no_such_profile(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_paths_agree_on_the_collection_they_serve() {
        // `ITEM_PATH` is what the router registers and what the OpenAPI document
        // publishes; `COLLECTION_PATH` is also what the `Location` header of a
        // creation is built from. They have to share a prefix or a client
        // following that header lands nowhere.
        assert!(ITEM_PATH.starts_with(COLLECTION_PATH));
        assert_eq!(&ITEM_PATH[COLLECTION_PATH.len()..], "/{id}");
    }
}
