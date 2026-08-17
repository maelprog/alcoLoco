//! HTTP entry point of alcoLoco.
//!
//! At this stage the server only exposes a liveness endpoint: business routes
//! are added by later issues.

use std::net::SocketAddr;

use axum::{Router, routing::get};

/// Default address the server binds to when `ALCOLOCO_API_ADDR` is not set.
const DEFAULT_ADDR: &str = "0.0.0.0:8080";

/// Liveness endpoint. Returns 200 as long as the process is able to serve.
async fn health() -> &'static str {
    "ok"
}

/// Builds the application router. Kept separate from `main` so that it can be
/// exercised by tests without binding a socket.
fn app() -> Router {
    Router::new().route("/health", get(health))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("ALCOLOCO_API_ADDR")
        .unwrap_or_else(|_| DEFAULT_ADDR.to_owned())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "alcoLoco api listening on http://{}",
        listener.local_addr()?
    );
    axum::serve(listener, app()).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_ok() {
        assert_eq!(health().await, "ok");
    }

    #[test]
    fn router_builds() {
        let _ = app();
    }
}
