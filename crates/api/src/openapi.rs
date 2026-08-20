//! OpenAPI document, generated from the handlers and the payload types.
//!
//! Generated rather than hand written: a hand-written document drifts from the
//! code silently, and the point of publishing one is that a reader can trust it.
//! It is served on [`DOCUMENT_PATH`] in development only (see
//! [`crate::config::Environment::exposes_openapi`]).

use axum::Json;
use utoipa::OpenApi;

/// Where the document is served, when it is served.
pub const DOCUMENT_PATH: &str = "/openapi.json";

/// The OpenAPI document of the alcoLoco API.
///
/// `ProblemDetails` and `FieldError` are registered even though no path names
/// them yet: they are the answer shape of every future failure, and #6 onwards
/// reference them rather than redeclare one each.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "alcoLoco",
        description = "Suivi d'alcoolémie — API interne.",
        version = "0.1.0",
    ),
    paths(crate::health::health),
    components(schemas(
        crate::error::FieldError,
        crate::error::ProblemDetails,
        crate::health::DatabaseStatus,
        crate::health::HealthResponse,
        crate::health::Status,
    )),
    tags((name = "system", description = "Exploitation: santé du service.")),
)]
pub struct ApiDoc;

/// Serves the OpenAPI document.
pub async fn document() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> serde_json::Value {
        serde_json::to_value(ApiDoc::openapi()).expect("the document must serialise")
    }

    #[test]
    fn the_document_describes_the_routes_the_router_serves() {
        let document = rendered();
        assert!(
            document["paths"]["/health"]["get"].is_object(),
            "GET /health is missing from the document: {document}"
        );
    }

    #[test]
    fn the_error_shape_is_published_for_every_later_endpoint_to_reference() {
        let document = rendered();
        let problem = &document["components"]["schemas"]["ProblemDetails"];
        let members = problem["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("ProblemDetails is not described: {document}"));
        for member in ["type", "title", "status", "detail", "errors"] {
            assert!(members.contains_key(member), "`{member}` is not documented");
        }
    }
}
