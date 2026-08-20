//! HTTP entry point of alcoLoco.
//!
//! ```text
//! DATABASE_URL       postgres://alcoloco:alcoloco@localhost:5432/alcoloco
//! ALCOLOCO_API_ADDR  0.0.0.0:8080
//! ALCOLOCO_LOG_LEVEL info
//! ALCOLOCO_ENV       development | production
//! ```
//!
//! Everything the process does lives in the `api` library; this binary only
//! wires the configuration to the router and serves it.
//!
//! `GET /health` always answers `200` and reports the state of the database in
//! its body. When the database is unreachable that answer costs the full probe
//! timeout, 2 s: a liveness check that gives up sooner records a timeout instead
//! of reading `"database": "down"`. Allow at least 3 s.

use std::process::ExitCode;

use api::{AppState, Config};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("api: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env()?;
    api::init_tracing(&config)?;

    let addr = config.addr;
    let environment = config.environment;
    let state = AppState::new(config)?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        address = %listener.local_addr()?,
        environment = %environment,
        "alcoLoco api listening"
    );

    axum::serve(listener, api::app(state)).await?;
    Ok(())
}
