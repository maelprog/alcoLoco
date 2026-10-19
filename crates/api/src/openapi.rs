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

/// The OpenAPI document of the alcoLoco API, as the derive builds it.
///
/// Private: the document leaves this module through [`spec`] only.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "alcoLoco",
        description = "Suivi d'alcoolémie — API interne.",
        version = "0.1.0",
    ),
    paths(
        crate::health::health,
        crate::profile::list,
        crate::profile::read,
        crate::profile::create,
        crate::profile::update,
    ),
    components(schemas(
        crate::error::FieldError,
        crate::error::ProblemDetails,
        crate::health::DatabaseStatus,
        crate::health::HealthResponse,
        crate::health::Status,
        crate::profile::Profile,
        crate::profile::ProfileRequest,
        crate::profile::ProfileSettings,
        crate::profile::ProfileUpdate,
        crate::profile::QuantityUnit,
        crate::profile::Sex,
        crate::profile::SettingsRequest,
    )),
    tags(
        (name = "system", description = "Exploitation: santé du service."),
        (name = "profiles", description = "Profils et paramètres physiologiques versionnés."),
    ),
)]
struct ApiDoc;

/// The document, as served.
///
/// `utoipa` fills `info.license` from the Cargo manifest, so the document
/// publishes the licence the workspace declares (`license = "MIT"`).
#[must_use]
pub fn spec() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// Serves the OpenAPI document.
pub async fn document() -> Json<utoipa::openapi::OpenApi> {
    Json(spec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> serde_json::Value {
        serde_json::to_value(spec()).expect("the document must serialise")
    }

    #[test]
    fn the_document_publishes_the_mit_licence() {
        // The workspace is licensed MIT, and the document says so. OpenAPI 3.1
        // requires `name` on a licence object, so an empty one would not do.
        let document = rendered();
        assert_eq!(
            document["info"]["license"]["name"], "MIT",
            "the document does not publish the MIT licence: {document}"
        );
    }

    #[test]
    fn the_document_describes_the_routes_the_router_serves() {
        // Every path the router registers, listed against the document rather
        // than spot checked: a handler mounted on `app` but left out of
        // `paths(...)` is invisible to a reader of the specification, and
        // nothing else in the workspace would notice.
        let document = rendered();
        for (path, methods) in [
            ("/health", &["get"][..]),
            (crate::profile::COLLECTION_PATH, &["get", "post"]),
            (crate::profile::ITEM_PATH, &["get", "put"]),
        ] {
            for method in methods {
                assert!(
                    document["paths"][path][method].is_object(),
                    "{} {path} is missing from the document: {document}",
                    method.to_uppercase()
                );
            }
        }
    }

    #[test]
    fn a_failing_endpoint_publishes_the_shared_error_shape() {
        // The reason `ProblemDetails` was registered by #3 before any path used
        // it. A response documented with a body of its own would give clients a
        // second error shape to parse, which `error.rs` exists to prevent.
        let document = rendered();
        let reference = serde_json::json!({ "$ref": "#/components/schemas/ProblemDetails" });
        for (path, method, status) in [
            (crate::profile::COLLECTION_PATH, "post", "400"),
            (crate::profile::ITEM_PATH, "get", "404"),
            (crate::profile::ITEM_PATH, "put", "409"),
        ] {
            let schema = &document["paths"][path][method]["responses"][status]["content"]["application/json"]
                ["schema"];
            assert_eq!(
                *schema,
                reference,
                "{} {path} answers {status} with something other than a problem document: {document}",
                method.to_uppercase()
            );
        }
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
