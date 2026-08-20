//! Integration tests that need a real PostgreSQL.
//!
//! There is no PostgreSQL service in CI (an acknowledged gap, tracked outside
//! this issue), so every test here is conditioned on `DATABASE_URL` and skips
//! itself, loudly, when it is not set — `cargo test --workspace` therefore stays
//! green on a machine without a database. They are not marked `#[ignore]`: an
//! ignored test never runs, not even where a database *is* available.
//!
//! Replay them against a database with:
//!
//! ```text
//! DATABASE_URL=postgres://alcoloco:alcoloco@localhost:5432/alcoloco \
//!   cargo test --workspace
//! ```

use api::config::{DATABASE_URL_ENV, ENVIRONMENT_ENV};
use api::{AppState, Config};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// The connection string to test against, or `None` when there is none.
///
/// Printed skips are what keeps this suite honest: a run without a database
/// says so on stdout instead of silently reporting the same "ok" as a run with
/// one.
fn database_url(test: &str) -> Option<String> {
    match std::env::var(DATABASE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            eprintln!("skipping `{test}`: {DATABASE_URL_ENV} is not set");
            None
        }
    }
}

fn state(database_url: &str) -> AppState {
    let database_url = database_url.to_owned();
    let config = Config::from_source(move |key| match key {
        DATABASE_URL_ENV => Some(database_url.clone()),
        ENVIRONMENT_ENV => Some("development".to_owned()),
        _ => None,
    })
    .expect("the test configuration must parse");
    AppState::new(config).expect("the connection string must parse")
}

async fn body_of(state: AppState, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request must build");
    let response = api::app(state)
        .oneshot(request)
        .await
        .expect("the router must answer");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body must be readable")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn health_reports_the_database_as_up_when_it_answers() {
    let Some(url) = database_url("health_reports_the_database_as_up_when_it_answers") else {
        return;
    };

    let (status, body) = body_of(state(&url), "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database"], "up", "{body}");
    assert_eq!(body["status"], "ok", "{body}");
}

#[tokio::test]
async fn the_pool_the_api_builds_reaches_the_migrated_schema() {
    // Proves three things at once against a real server: the configuration
    // reaches sqlx, the migrations of the `db` crate apply, and the schema they
    // leave behind is the one the later endpoints will query. No Rust gate
    // touches the database, so nothing else in the workspace would notice a
    // migration that no longer applies.
    let name = "the_pool_the_api_builds_reaches_the_migrated_schema";
    let Some(url) = database_url(name) else {
        return;
    };

    let state = state(&url);
    db::migrate(&state.pool)
        .await
        .expect("the migrations must apply");

    for table in ["profile", "profile_settings_version", "drink", "event"] {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.tables
                 WHERE table_schema = 'public' AND table_name = $1
             )",
        )
        .bind(table)
        .fetch_one(&state.pool)
        .await
        .expect("the probe query must run");
        assert!(
            present,
            "table `{table}` is missing from the migrated schema"
        );
    }
}
