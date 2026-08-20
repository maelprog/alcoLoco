//! The single error type of the API and its RFC 7807 rendering.
//!
//! Every failure leaving a handler is an [`ApiError`], and every [`ApiError`]
//! renders as the same document: `application/problem+json` with the members
//! `type`, `title`, `status`, `detail` and `errors`. A business failure and an
//! internal failure differ by their status code and their wording, never by
//! their shape — a client can therefore parse one payload and be done.
//!
//! `errors` is always present, empty when nothing field-specific is being
//! reported. Omitting it when empty would make a validation failure carry one
//! more member than a not-found, which is exactly the shape difference this
//! module exists to prevent.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// Media type of every error payload (RFC 7807 §3).
pub const PROBLEM_JSON: &str = "application/problem+json";

/// Prefix of the `type` member. Relative references are allowed by RFC 7807 §3.1
/// and keep the identifiers stable across deployments and hostnames.
const PROBLEM_TYPE_PREFIX: &str = "/problems/";

/// One field-level validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct FieldError {
    /// Name of the offending member, as spelled in the request payload.
    pub field: String,
    /// Human readable explanation of what is wrong with it.
    pub detail: String,
}

impl FieldError {
    /// Builds a field error.
    pub fn new(field: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            detail: detail.into(),
        }
    }
}

/// The body of every error response (RFC 7807 problem details).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ProblemDetails {
    /// Identifier of the problem kind, e.g. `/problems/not_found`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Short, human readable summary of the problem kind.
    pub title: String,
    /// HTTP status code, repeated in the body as RFC 7807 prescribes.
    pub status: u16,
    /// Explanation specific to this occurrence.
    pub detail: String,
    /// Field-level failures. Empty for every problem that is not a validation
    /// failure — never absent, so that the payload shape never varies.
    pub errors: Vec<FieldError>,
}

/// Every failure an handler may return.
#[derive(Debug)]
pub enum ApiError {
    /// The request itself is malformed: unusable query string, unreadable body.
    BadRequest(String),
    /// The addressed resource does not exist.
    NotFound(String),
    /// The resource exists but not under this method.
    MethodNotAllowed(String),
    /// The request cannot be applied to the current state of the resource.
    Conflict(String),
    /// The request is well formed but its content is invalid, field by field.
    Validation(Vec<FieldError>),
    /// Anything that is the server's fault. The cause is logged, never sent.
    Internal(Box<dyn std::error::Error + Send + Sync>),
}

impl ApiError {
    /// A malformed request.
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::BadRequest(detail.into())
    }

    /// A missing resource.
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::NotFound(detail.into())
    }

    /// A conflict with the current state.
    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::Conflict(detail.into())
    }

    /// A method the addressed resource does not answer.
    pub fn method_not_allowed(detail: impl Into<String>) -> Self {
        Self::MethodNotAllowed(detail.into())
    }

    /// A field-by-field validation failure.
    #[must_use]
    pub fn validation(errors: Vec<FieldError>) -> Self {
        Self::Validation(errors)
    }

    /// A server-side failure. The cause is kept for the logs only.
    pub fn internal(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Internal(cause.into())
    }

    /// HTTP status this failure maps to.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed(_) => StatusCode::METHOD_NOT_ALLOWED,
            Self::Conflict(_) => StatusCode::CONFLICT,
            // 400 rather than 422: the project already answers 400 to an
            // out-of-range query parameter (SPEC.md §10.0-I), and one status for
            // "you sent something invalid" is easier to hold than two.
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable slug of the problem kind, used to build the `type` member.
    const fn slug(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::MethodNotAllowed(_) => "method_not_allowed",
            Self::Conflict(_) => "conflict",
            Self::Validation(_) => "validation_failed",
            Self::Internal(_) => "internal_error",
        }
    }

    /// Short summary of the problem kind.
    const fn title(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "Malformed request",
            Self::NotFound(_) => "Resource not found",
            Self::MethodNotAllowed(_) => "Method not allowed",
            Self::Conflict(_) => "Conflicting request",
            Self::Validation(_) => "Validation failed",
            Self::Internal(_) => "Internal server error",
        }
    }

    /// Renders the failure as a problem document.
    ///
    /// The cause of an internal failure never reaches this document: it may
    /// carry a connection string or a SQL fragment. It is logged instead, by
    /// [`IntoResponse::into_response`].
    #[must_use]
    pub fn to_problem_details(&self) -> ProblemDetails {
        let detail = match self {
            Self::BadRequest(detail)
            | Self::NotFound(detail)
            | Self::MethodNotAllowed(detail)
            | Self::Conflict(detail) => detail.clone(),
            Self::Validation(errors) => {
                format!("the request payload is invalid ({} field(s))", errors.len())
            }
            Self::Internal(_) => "the request could not be processed".to_owned(),
        };

        ProblemDetails {
            kind: format!("{PROBLEM_TYPE_PREFIX}{}", self.slug()),
            title: self.title().to_owned(),
            status: self.status().as_u16(),
            detail,
            errors: match self {
                Self::Validation(errors) => errors.clone(),
                _ => Vec::new(),
            },
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(detail)
            | Self::NotFound(detail)
            | Self::MethodNotAllowed(detail)
            | Self::Conflict(detail) => formatter.write_str(detail),
            Self::Validation(errors) => write!(formatter, "{} invalid field(s)", errors.len()),
            Self::Internal(cause) => write!(formatter, "internal error: {cause}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(cause) = &self {
            tracing::error!(error = %cause, "request failed with an internal error");
        }
        let status = self.status();
        let mut response = (status, Json(self.to_problem_details())).into_response();
        // `Json` labels the body `application/json`; RFC 7807 requires the
        // problem media type, and it is what tells a client the body follows
        // this shape rather than the shape of a successful answer.
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(problem: &ProblemDetails) -> Vec<String> {
        let value = serde_json::to_value(problem).expect("a problem must serialise");
        let mut keys: Vec<String> = value
            .as_object()
            .expect("a problem serialises as an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn every_problem_carries_exactly_the_rfc_7807_members() {
        let expected = vec![
            "detail".to_owned(),
            "errors".to_owned(),
            "status".to_owned(),
            "title".to_owned(),
            "type".to_owned(),
        ];
        for error in [
            ApiError::bad_request("nope"),
            ApiError::not_found("nope"),
            ApiError::method_not_allowed("nope"),
            ApiError::conflict("nope"),
            ApiError::validation(vec![FieldError::new("weight_kg", "must be positive")]),
            ApiError::internal("boom"),
        ] {
            assert_eq!(members(&error.to_problem_details()), expected, "{error}");
        }
    }

    #[test]
    fn the_body_status_repeats_the_http_status() {
        for error in [
            ApiError::bad_request("nope"),
            ApiError::not_found("nope"),
            ApiError::method_not_allowed("nope"),
            ApiError::conflict("nope"),
            ApiError::validation(Vec::new()),
            ApiError::internal("boom"),
        ] {
            let status = error.status();
            assert_eq!(error.to_problem_details().status, status.as_u16());
        }
    }

    #[test]
    fn distinct_kinds_get_distinct_type_identifiers() {
        let kinds: Vec<String> = [
            ApiError::bad_request(""),
            ApiError::not_found(""),
            ApiError::method_not_allowed(""),
            ApiError::conflict(""),
            ApiError::validation(Vec::new()),
            ApiError::internal("boom"),
        ]
        .iter()
        .map(|error| error.to_problem_details().kind)
        .collect();

        let mut unique = kinds.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            kinds.len(),
            "ambiguous problem types: {kinds:?}"
        );
        assert!(
            kinds
                .iter()
                .all(|kind| kind.starts_with(PROBLEM_TYPE_PREFIX))
        );
    }

    #[test]
    fn the_cause_of_an_internal_failure_never_reaches_the_payload() {
        let secret = "postgres://alcoloco:hunter2@db.internal/alcoloco is unreachable";
        let problem = ApiError::internal(secret).to_problem_details();
        let rendered = serde_json::to_string(&problem).expect("must serialise");
        assert!(
            !rendered.contains("hunter2"),
            "the cause leaked into the payload: {rendered}"
        );
    }

    #[test]
    fn validation_reports_the_offending_fields() {
        let problem = ApiError::validation(vec![
            FieldError::new("limit", "must be between 1 and 200"),
            FieldError::new("cursor", "must be a UUID"),
        ])
        .to_problem_details();

        assert_eq!(problem.errors.len(), 2);
        assert_eq!(problem.errors[0].field, "limit");
        assert_eq!(problem.errors[1].field, "cursor");
    }

    #[test]
    fn a_database_failure_becomes_an_internal_error() {
        let error: ApiError = sqlx::Error::PoolClosed.into();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
