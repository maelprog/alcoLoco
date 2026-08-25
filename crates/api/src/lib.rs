//! HTTP layer of alcoLoco.
//!
//! This crate holds what every endpoint needs and nothing that belongs to a
//! single one: configuration, connection pool, the error type and its RFC 7807
//! rendering, the identifier and timestamp conventions, cursor pagination, and
//! the generated OpenAPI document. Business routes live in a module of their own
//! and are mounted on [`app`] — [`profile`] is the first of them.
//!
//! Conventions posed here, to be reused rather than restated:
//!
//! - identifiers are UUID v7 minted by the application ([`id::new_id`]);
//! - failures answer `application/problem+json` ([`error::ApiError`]);
//! - collections page by cursor ([`pagination`]), never by offset;
//! - instants travel as ISO 8601 UTC with a `Z` suffix ([`timestamp::Timestamp`]);
//! - JSON members are `snake_case`, spelled like the columns behind them.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{Method, Uri};
use axum::routing::get;
use sqlx::PgPool;

pub mod config;
pub mod error;
pub mod extract;
pub mod health;
pub mod id;
pub mod openapi;
pub mod pagination;
pub mod profile;
pub mod timestamp;

pub use config::{Config, Environment};
pub use error::{ApiError, FieldError, ProblemDetails};
pub use id::new_id;
pub use pagination::{Page, PageQuery, PageRequest};
pub use profile::{Profile, ProfileSettings};
pub use timestamp::Timestamp;

/// How long a handler waits for a free connection before giving up.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on the connections the API holds open.
const MAX_CONNECTIONS: u32 = 16;

/// What every handler is given.
#[derive(Clone)]
pub struct AppState {
    /// Connection pool on the configured database.
    pub pool: PgPool,
    /// Configuration read at boot. Shared, never re-read.
    pub config: Arc<Config>,
}

impl AppState {
    /// Builds the state from a configuration, without contacting the database.
    ///
    /// The pool is lazy on purpose: the process must come up, bind, and answer
    /// `GET /health` with `database: "down"` even when PostgreSQL is not there.
    /// An eager pool would turn a database outage into a server that refuses to
    /// start, and therefore into a health endpoint nobody can query.
    ///
    /// # Errors
    ///
    /// Fails only when the connection string cannot be parsed.
    pub fn new(config: Config) -> Result<Self, sqlx::Error> {
        let pool = db::pool_options()
            .max_connections(MAX_CONNECTIONS)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_lazy(&config.database_url)?;
        Ok(Self {
            pool,
            config: Arc::new(config),
        })
    }
}

/// Builds the router.
///
/// Kept separate from the binary so that tests drive the very same routes the
/// server serves, without binding a socket.
pub fn app(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/health", get(health::health))
        .route(
            profile::COLLECTION_PATH,
            get(profile::list).post(profile::create),
        )
        .route(profile::ITEM_PATH, get(profile::read).put(profile::update));

    if state.config.environment.exposes_openapi() {
        router = router.route(openapi::DOCUMENT_PATH, get(openapi::document));
    }

    router
        .fallback(unknown_route)
        .method_not_allowed_fallback(wrong_method)
        .with_state(state)
}

/// Answer to a path no route claims — a problem document, like every other
/// failure, rather than axum's empty 404.
async fn unknown_route(uri: Uri) -> ApiError {
    ApiError::not_found(format!("no resource at {}", uri.path()))
}

/// Answer to a known path addressed with a method it does not serve.
async fn wrong_method(method: Method, uri: Uri) -> ApiError {
    ApiError::method_not_allowed(format!("{} does not answer {method}", uri.path()))
}

/// Installs the log subscriber for the level the configuration asked for.
///
/// # Errors
///
/// Fails when the level is not a valid `tracing-subscriber` filter, or when a
/// subscriber is already installed.
pub fn init_tracing(config: &Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)?;
    tracing_subscriber::fmt().with_env_filter(filter).try_init()
}
