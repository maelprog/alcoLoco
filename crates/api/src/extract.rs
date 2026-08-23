//! Extractors that fail the way the rest of the API fails.
//!
//! `axum`'s own `Query` and `Json` reject a malformed request with a plain-text
//! body, so a client parsing `application/problem+json` would meet a payload it
//! cannot read precisely when something went wrong. These wrappers delegate the
//! parsing and only rewrite the rejection, so that *every* 4xx the API produces
//! is a problem document.

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// Query-string extractor rejecting with a problem document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|rejection| ApiError::bad_request(rejection.body_text()))
    }
}

/// Path-parameter extractor rejecting with a problem document.
///
/// `GET /profiles/not-a-uuid` is a malformed request, not a missing resource:
/// answering 404 would tell a client the identifier could have existed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Self(value))
            .map_err(|rejection| ApiError::bad_request(rejection.body_text()))
    }
}

/// JSON body extractor rejecting with a problem document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(request, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|rejection| ApiError::bad_request(rejection.body_text()))
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::{get, post};
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;
    use crate::error::PROBLEM_JSON;

    #[derive(Debug, Deserialize)]
    struct Wanted {
        #[allow(dead_code)]
        weight_kg: f64,
    }

    fn router() -> Router {
        Router::new()
            .route("/q", get(|Query(_): Query<Wanted>| async { "ok" }))
            .route("/j", post(|Json(_): Json<Wanted>| async { "ok" }))
            .route("/p/{id}", get(|Path(_): Path<uuid::Uuid>| async { "ok" }))
    }

    async fn media_type_of(request: Request<Body>) -> (StatusCode, String) {
        let response = router()
            .oneshot(request)
            .await
            .expect("the router must answer");
        let status = response.status();
        let media_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        (status, media_type)
    }

    #[tokio::test]
    async fn a_query_string_that_does_not_fit_is_rejected_as_a_problem() {
        let request = Request::builder()
            .uri("/q?weight_kg=heavy")
            .body(Body::empty())
            .expect("must build");
        let (status, media_type) = media_type_of(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(media_type, PROBLEM_JSON);
    }

    #[tokio::test]
    async fn a_body_that_does_not_fit_is_rejected_as_a_problem() {
        let request = Request::builder()
            .method("POST")
            .uri("/j")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"weight_kg":"heavy"}"#))
            .expect("must build");
        let (status, media_type) = media_type_of(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(media_type, PROBLEM_JSON);
    }

    #[tokio::test]
    async fn a_path_parameter_that_does_not_fit_is_rejected_as_a_problem() {
        // 400 and not 404: the identifier is unusable, which is a different
        // thing from naming a resource that does not exist.
        let request = Request::builder()
            .uri("/p/not-a-uuid")
            .body(Body::empty())
            .expect("must build");
        let (status, media_type) = media_type_of(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(media_type, PROBLEM_JSON);
    }

    #[tokio::test]
    async fn a_request_that_fits_reaches_the_handler() {
        let request = Request::builder()
            .uri("/q?weight_kg=62.5")
            .body(Body::empty())
            .expect("must build");
        let (status, _) = media_type_of(request).await;
        assert_eq!(status, StatusCode::OK);
    }
}
