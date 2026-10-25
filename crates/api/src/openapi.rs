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
        crate::library::list,
        crate::drink::list,
        crate::drink::create,
        crate::drink::read,
        crate::drink::update,
        crate::drink::delete,
        crate::event::list,
        crate::event::read,
        crate::event::create,
        crate::event::update,
        crate::event::delete,
        crate::event::add_participant,
        crate::event::remove_participant,
        crate::bac::profile,
        crate::bac::event,
        crate::bac::feeling_suggestion,
    ),
    components(schemas(
        crate::bac::EventBac,
        crate::bac::FeelingSuggestion,
        crate::bac::ParticipantBac,
        crate::bac::ProfileBac,
        crate::drink::Component,
        crate::drink::ComponentRequest,
        crate::drink::Drink,
        crate::drink::DrinkPatch,
        crate::drink::DrinkRequest,
        crate::error::FieldError,
        crate::event::Event,
        crate::event::EventRequest,
        crate::event::Participant,
        crate::error::ProblemDetails,
        crate::health::DatabaseStatus,
        crate::health::HealthResponse,
        crate::health::Status,
        crate::library::Category,
        crate::library::LibraryItem,
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
        (name = "library", description = "Bibliothèque de base, en lecture seule."),
        (name = "drinks", description = "Consommations d'un profil : saisie, historique, modification, suppression."),
        (name = "events", description = "Événements et profils participants : création, lecture, modification, suppression."),
        (name = "curves", description = "Courbes d'alcoolémie d'un profil et des participants d'un événement."),
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
            (crate::library::COLLECTION_PATH, &["get"]),
            (crate::drink::PROFILE_COLLECTION_PATH, &["get", "post"]),
            (crate::drink::ITEM_PATH, &["get", "patch", "delete"]),
            (crate::event::COLLECTION_PATH, &["get", "post"]),
            (crate::event::ITEM_PATH, &["get", "put", "delete"]),
            (crate::event::PARTICIPANT_PATH, &["put", "delete"]),
            (crate::bac::PROFILE_PATH, &["get"]),
            (crate::bac::EVENT_PATH, &["get"]),
            (crate::bac::FEELING_SUGGESTION_PATH, &["get"]),
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
            (crate::library::COLLECTION_PATH, "get", "400"),
            (crate::drink::PROFILE_COLLECTION_PATH, "post", "404"),
            (crate::drink::PROFILE_COLLECTION_PATH, "get", "400"),
            (crate::drink::PROFILE_COLLECTION_PATH, "get", "404"),
            (crate::drink::ITEM_PATH, "patch", "400"),
            (crate::drink::ITEM_PATH, "delete", "404"),
            (crate::event::COLLECTION_PATH, "post", "400"),
            (crate::event::ITEM_PATH, "put", "404"),
            (crate::event::ITEM_PATH, "delete", "400"),
            (crate::event::ITEM_PATH, "delete", "404"),
            (crate::event::PARTICIPANT_PATH, "put", "404"),
            (crate::event::PARTICIPANT_PATH, "delete", "404"),
            (crate::bac::FEELING_SUGGESTION_PATH, "get", "400"),
            (crate::bac::FEELING_SUGGESTION_PATH, "get", "404"),
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
    fn a_drink_patch_publishes_which_members_may_be_null() {
        // Every member is optional; only the three the API clears on `null`
        // are documented as nullable, the others answer 400 to it.
        let document = rendered();
        let patch = &document["components"]["schemas"]["DrinkPatch"];
        let required = patch["required"].as_array().cloned().unwrap_or_default();
        assert!(required.is_empty(), "a patch requires nothing: {patch}");
        let nullable = |member: &str| {
            let schema = &patch["properties"][member];
            let rendered = schema.to_string();
            assert!(schema.is_object(), "`{member}` is not documented: {patch}");
            rendered.contains("\"null\"") || schema["nullable"] == true
        };
        for member in ["event_id", "volume_total_ml", "feeling_rating"] {
            assert!(nullable(member), "`{member}` clears on null: {patch}");
        }
        for member in [
            "name",
            "ingested_at",
            "ingestion_duration_seconds",
            "absorption_duration_seconds",
            "quantity_unit",
            "components",
        ] {
            assert!(!nullable(member), "`{member}` refuses null: {patch}");
        }
    }

    /// The paths that serve a [`crate::pagination::Page`].
    const PAGED_PATHS: [&str; 4] = [
        crate::profile::COLLECTION_PATH,
        crate::library::COLLECTION_PATH,
        crate::drink::PROFILE_COLLECTION_PATH,
        crate::event::COLLECTION_PATH,
    ];

    /// The query parameter `name` of `GET path`.
    fn query_parameter(document: &serde_json::Value, path: &str, name: &str) -> serde_json::Value {
        document["paths"][path]["get"]["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("GET {path} declares no parameter: {document}"))
            .iter()
            .find(|parameter| parameter["name"] == name && parameter["in"] == "query")
            .cloned()
            .unwrap_or_else(|| panic!("GET {path} does not declare `?{name}=`: {document}"))
    }

    #[test]
    fn the_page_size_is_published_as_a_bounded_integer_with_its_default() {
        // `limit` is read as text, so that `?limit=abc` is a field error like
        // `?limit=1000`; left to the derive, it was published as a string,
        // without the bounds a client has to respect. The bounds are compared
        // with the constants the validation uses, not with numbers written
        // here.
        let document = rendered();
        for path in PAGED_PATHS {
            let limit = query_parameter(&document, path, "limit");
            let schema = &limit["schema"];
            assert_eq!(schema["type"], "integer", "GET {path}: {limit}");
            assert_eq!(schema["minimum"], 1, "GET {path}: {limit}");
            assert_eq!(
                schema["maximum"],
                crate::pagination::MAX_LIMIT,
                "GET {path}: {limit}"
            );
            assert_eq!(
                schema["default"],
                crate::pagination::DEFAULT_LIMIT,
                "GET {path}: {limit}"
            );
            let description = limit["description"].as_str().unwrap_or_default();
            assert!(
                !description.contains('['),
                "GET {path}: a rustdoc link leaked into the description: {limit}"
            );
            let cursor = query_parameter(&document, path, "cursor");
            assert_eq!(cursor["schema"]["type"], "string", "GET {path}: {cursor}");
        }
    }

    #[test]
    fn a_page_always_carries_its_next_cursor() {
        // `{ "items": [...], "next_cursor": null }` on the last page: the
        // member is never absent, so the schema requires it and lets it be
        // null.
        let document = rendered();
        for path in PAGED_PATHS {
            let schema = &document["paths"][path]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"];
            let page = match schema["$ref"].as_str() {
                Some(reference) => {
                    let name = reference.trim_start_matches("#/components/schemas/");
                    document["components"]["schemas"][name].clone()
                }
                None => schema.clone(),
            };
            let required: Vec<&str> = page["required"]
                .as_array()
                .unwrap_or_else(|| panic!("GET {path}: a page requires nothing: {page}"))
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            assert!(
                required.contains(&"items") && required.contains(&"next_cursor"),
                "GET {path}: {page}"
            );
            let next_cursor = page["properties"]["next_cursor"].to_string();
            assert!(
                next_cursor.contains("\"null\"") || next_cursor.contains("\"nullable\":true"),
                "GET {path}: `next_cursor` is null on the last page: {page}"
            );
        }
    }

    #[test]
    fn the_history_publishes_its_filters_with_their_formats() {
        let document = rendered();
        let path = crate::drink::PROFILE_COLLECTION_PATH;
        for name in ["from", "before"] {
            let parameter = query_parameter(&document, path, name);
            assert_eq!(parameter["schema"]["type"], "string", "{parameter}");
            assert_eq!(parameter["schema"]["format"], "date-time", "{parameter}");
            assert_ne!(parameter["required"], true, "{parameter}");
        }
        let event = query_parameter(&document, path, "event_id");
        assert_eq!(event["schema"]["format"], "uuid", "{event}");
        assert_ne!(event["required"], true, "{event}");
    }

    #[test]
    fn the_feeling_suggestion_publishes_its_optional_instant_and_its_bounded_rating() {
        let document = rendered();
        let path = crate::bac::FEELING_SUGGESTION_PATH;
        let parameter = query_parameter(&document, path, "at");
        assert_eq!(parameter["schema"]["format"], "date-time", "{parameter}");
        assert_ne!(parameter["required"], true, "{parameter}");
        let suggestion = &document["components"]["schemas"]["FeelingSuggestion"];
        let rating = &suggestion["properties"]["rating"];
        assert_eq!(rating["minimum"], 1, "{suggestion}");
        assert_eq!(
            rating["maximum"],
            domain::feeling::SUGGESTED_RATING_MAX,
            "{suggestion}"
        );
        let mut required: Vec<&str> = suggestion["required"]
            .as_array()
            .unwrap_or_else(|| panic!("a suggestion requires nothing: {suggestion}"))
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        required.sort_unstable();
        assert_eq!(required, ["at", "bac_g_per_l", "rating"], "{suggestion}");
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
