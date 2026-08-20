//! End-to-end behaviour of the router, driven in process.
//!
//! None of these tests needs a database: the state is built on a connection
//! string pointing at a closed port, which is precisely what lets the "database
//! unreachable" branch of `GET /health` be exercised here rather than only in
//! the database-backed suite.

use api::config::{DATABASE_URL_ENV, ENVIRONMENT_ENV};
use api::error::PROBLEM_JSON;
use api::{ApiError, AppState, Config};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// A port nothing listens on, so the pool fails fast rather than hanging.
const UNREACHABLE_DATABASE: &str = "postgres://alcoloco:alcoloco@127.0.0.1:1/alcoloco";

fn state(environment: &str) -> AppState {
    let environment = environment.to_owned();
    let config = Config::from_source(move |key| match key {
        DATABASE_URL_ENV => Some(UNREACHABLE_DATABASE.to_owned()),
        ENVIRONMENT_ENV => Some(environment.clone()),
        _ => None,
    })
    .expect("the test configuration must parse");
    AppState::new(config).expect("the connection string must parse")
}

struct Answer {
    status: StatusCode,
    media_type: String,
    body: Value,
}

async fn call(router: Router, method: &str, uri: &str) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("the request must build");
    let response = router
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
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body must be readable")
        .to_bytes();

    Answer {
        status,
        media_type,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn members(body: &Value) -> Vec<String> {
    let mut keys: Vec<String> = body
        .as_object()
        .unwrap_or_else(|| panic!("expected a JSON object, got {body}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// Two routes whose only job is to fail, one the caller's fault and one ours.
fn failing_router() -> Router {
    Router::new()
        .route(
            "/business",
            get(|| async {
                Err::<(), ApiError>(ApiError::not_found("no profile with that identifier"))
            }),
        )
        .route(
            "/internal",
            get(|| async { Err::<(), ApiError>(ApiError::internal("connection reset by peer")) }),
        )
        .route(
            "/validation",
            get(|| async {
                Err::<(), ApiError>(ApiError::validation(vec![api::FieldError::new(
                    "weight_kg",
                    "must be positive",
                )]))
            }),
        )
}

#[tokio::test]
async fn a_business_failure_and_an_internal_failure_answer_the_same_shape() {
    // Acceptance criterion of issue #3. What is compared is the *shape*: the
    // media type and the exact set of members. The status and the wording are
    // expected to differ — that is the whole point of having two of them.
    let business = call(failing_router(), "GET", "/business").await;
    let internal = call(failing_router(), "GET", "/internal").await;

    assert_eq!(business.status, StatusCode::NOT_FOUND);
    assert_eq!(internal.status, StatusCode::INTERNAL_SERVER_ERROR);

    assert_eq!(business.media_type, PROBLEM_JSON);
    assert_eq!(internal.media_type, PROBLEM_JSON);

    assert_eq!(
        members(&business.body),
        members(&internal.body),
        "business {} vs internal {}",
        business.body,
        internal.body
    );
    assert_eq!(
        members(&business.body),
        vec![
            "detail".to_owned(),
            "errors".to_owned(),
            "status".to_owned(),
            "title".to_owned(),
            "type".to_owned(),
        ]
    );

    // Each member is the same JSON type on both sides: identical keys carrying
    // a string here and an object there would not be one shape.
    for member in members(&business.body) {
        assert_eq!(
            std::mem::discriminant(&business.body[&member]),
            std::mem::discriminant(&internal.body[&member]),
            "member `{member}` has a different type on each side",
        );
    }

    assert_eq!(business.body["status"], 404);
    assert_eq!(internal.body["status"], 500);
    assert_ne!(business.body["type"], internal.body["type"]);
}

#[tokio::test]
async fn a_validation_failure_answers_that_same_shape_too() {
    // The other reading of "business error": a field-by-field rejection. It
    // fills `errors` instead of leaving it empty, and that is the *only*
    // difference — `errors` is serialised even when empty precisely so that
    // this payload does not carry one member more than the others.
    let validation = call(failing_router(), "GET", "/validation").await;
    let internal = call(failing_router(), "GET", "/internal").await;

    assert_eq!(validation.status, StatusCode::BAD_REQUEST);
    assert_eq!(validation.media_type, PROBLEM_JSON);
    assert_eq!(
        members(&validation.body),
        members(&internal.body),
        "validation {} vs internal {}",
        validation.body,
        internal.body
    );

    assert_eq!(validation.body["errors"][0]["field"], "weight_kg");
    assert_eq!(
        internal.body["errors"],
        serde_json::json!([]),
        "`errors` must stay present and empty outside validation: {}",
        internal.body
    );
}

#[tokio::test]
async fn an_internal_failure_says_nothing_about_its_cause() {
    let answer = call(failing_router(), "GET", "/internal").await;
    let rendered = answer.body.to_string();
    assert!(
        !rendered.contains("connection reset"),
        "the cause reached the client: {rendered}"
    );
}

#[tokio::test]
async fn health_answers_200_and_reports_an_unreachable_database() {
    let answer = call(api::app(state("development")), "GET", "/health").await;

    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.body["status"], "degraded");
    assert_eq!(answer.body["database"], "down");
    assert!(
        answer.body["checked_at"]
            .as_str()
            .is_some_and(|at| at.ends_with('Z')),
        "checked_at must be ISO 8601 UTC: {}",
        answer.body
    );
}

#[tokio::test]
async fn the_openapi_document_is_served_in_development() {
    let answer = call(api::app(state("development")), "GET", "/openapi.json").await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer.body["paths"]["/health"]["get"].is_object(),
        "the served document does not describe GET /health: {}",
        answer.body
    );
}

#[tokio::test]
async fn the_openapi_document_is_not_served_in_production() {
    let answer = call(api::app(state("production")), "GET", "/openapi.json").await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.media_type, PROBLEM_JSON);
}

#[tokio::test]
async fn an_unknown_path_answers_a_problem_document() {
    let answer = call(api::app(state("development")), "GET", "/nope").await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.media_type, PROBLEM_JSON);
    assert_eq!(answer.body["status"], 404);
}

#[tokio::test]
async fn a_method_the_route_does_not_serve_answers_a_problem_document() {
    let answer = call(api::app(state("development")), "DELETE", "/health").await;
    assert_eq!(answer.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(answer.media_type, PROBLEM_JSON);
    assert_eq!(answer.body["status"], 405);
}
