//! `GET /health` — liveness of the process and reachability of the database.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::timestamp::{self, Timestamp};

/// How long the database probe is given before it is called unreachable. Short
/// on purpose: a health endpoint that hangs is indistinguishable from a dead
/// process for the probe watching it.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The process serves and every dependency answers.
    Ok,
    /// The process serves but a dependency does not answer.
    Degraded,
}

/// State of the database connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseStatus {
    /// A pooled connection answered the probe query.
    Up,
    /// No connection could be obtained, or the probe query failed.
    Down,
}

/// Body of `GET /health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Overall verdict.
    pub status: Status,
    /// State of the database connection.
    pub database: DatabaseStatus,
    /// Instant the probe ran, ISO 8601 UTC.
    ///
    /// The schema is spelled out because `utoipa` recognises `chrono` types by
    /// their written name and cannot see through the [`Timestamp`] alias.
    #[schema(value_type = String, format = DateTime, example = "2026-08-20T21:04:05Z")]
    pub checked_at: Timestamp,
}

impl HealthResponse {
    /// Builds the answer matching a probe outcome.
    #[must_use]
    pub fn of(database: DatabaseStatus) -> Self {
        Self {
            status: match database {
                DatabaseStatus::Up => Status::Ok,
                DatabaseStatus::Down => Status::Degraded,
            },
            database,
            checked_at: timestamp::now(),
        }
    }
}

/// Reports whether the process serves and whether the database answers.
///
/// The answer is always `200`: the endpoint exists to report the state of the
/// dependencies as data. Turning an unreachable database into a failed request
/// would make "the process is gone" and "the process is up, the database is
/// not" indistinguishable to whoever is probing — which is the one distinction
/// the endpoint is for. That is also why the body is not a problem document:
/// nothing failed, the answer *is* the state.
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses((
        status = 200,
        description = "State of the process and of its database connection",
        body = HealthResponse,
    )),
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse::of(probe(&state.pool).await))
}

/// Runs the cheapest query that proves a connection can be obtained and used.
async fn probe(pool: &sqlx::PgPool) -> DatabaseStatus {
    let query = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool);
    match tokio::time::timeout(PROBE_TIMEOUT, query).await {
        Ok(Ok(1)) => DatabaseStatus::Up,
        Ok(Ok(other)) => {
            tracing::error!(
                answer = other,
                "the database answered `SELECT 1` with {other}"
            );
            DatabaseStatus::Down
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "database probe failed");
            DatabaseStatus::Down
        }
        Err(_) => {
            tracing::warn!(timeout = ?PROBE_TIMEOUT, "database probe timed out");
            DatabaseStatus::Down
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_database_degrades_the_overall_status() {
        assert_eq!(
            HealthResponse::of(DatabaseStatus::Down).status,
            Status::Degraded
        );
        assert_eq!(HealthResponse::of(DatabaseStatus::Up).status, Status::Ok);
    }

    #[test]
    fn the_payload_uses_the_agreed_member_and_value_spelling() {
        let body =
            serde_json::to_value(HealthResponse::of(DatabaseStatus::Down)).expect("must serialise");
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["database"], "down");
        assert!(
            body["checked_at"]
                .as_str()
                .is_some_and(|at| at.ends_with('Z')),
            "checked_at must be ISO 8601 UTC: {body}"
        );
    }
}
