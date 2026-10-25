//! What the front and the back have to spell identically: the problem
//! identifiers below, then the pagination bounds, then the feeling
//! suggestion, then the event deletion, then the profile deletion at the end
//! of the file.
//!
//! `web/src/app/core/api/problem-details.ts` mirrors the problem identifiers of
//! `crates/api/src/error.rs`. RFC 7807 makes `type` the stable member and the
//! front branches on it alone — `title` is English prose that may be reworded at
//! any time — so those six strings are the whole error contract between the two
//! sides.
//!
//! Until this file existed they were **transcribed by hand and guarded by
//! nothing**. Measured by the verification of PR #46 on 2026-08-21: rewriting
//! two of them in the TypeScript (`/problems/introuvable`, `/problems/CONFLICT`)
//! left the front lint green and its 46 tests green. What breaks when they drift
//! is not loud either: every real failure of the API falls through to the generic
//! "Une erreur inattendue est survenue.", which is the exact opposite of the
//! contract issue #4 set out to establish.
//!
//! The test that *looked* like it covered this, `api-error.spec.ts`, enumerates
//! the front's own constants; it can see neither a transcription error nor a
//! seventh kind added on the backend. For the problem identifiers and the
//! pagination bounds, this file reads **both sources** and compares them, so
//! it sees both.
//!
//! The feeling suggestion section checks the back alone, until the form of
//! #29 ships the front side: every rating `GET /profiles/{id}/feeling-suggestion`
//! can suggest is one the drink endpoints accept as `feeling_rating`, and its
//! payload travels as `{ at, bac_g_per_l, rating }`. No TypeScript is read
//! there yet, so a drift on the front side goes unnoticed.
//!
//! The event deletion section checks the back alone too, until the screen of
//! #23 ships: a drink detached from a deleted event still travels with its
//! `event_id` member, `null`.
//!
//! The profile deletion section reads both sides: a deleted profile is read
//! back with the problem the front names `ProblemType.notFound`.
//!
//! It lives on the Rust side for one reason: `include_str!` gives a compile-time
//! read of a file two directories up, and the Rust gate runs on every pull
//! request whether or not `web/` was touched. A drift introduced from either
//! side fails `cargo test --workspace`.

use std::collections::BTreeSet;

/// The module the identifiers are minted in.
const RUST_SOURCE: &str = include_str!("../src/error.rs");

/// The module that mirrors them for the front.
const TYPESCRIPT_SOURCE: &str = include_str!("../../../web/src/app/core/api/problem-details.ts");

/// The text between the first `open` after `from` and the next `close`.
fn between<'a>(haystack: &'a str, from: &str, open: &str, close: &str) -> &'a str {
    let at = haystack
        .find(from)
        .unwrap_or_else(|| panic!("`{from}` is gone from its source file"));
    let rest = &haystack[at + from.len()..];
    let start = rest
        .find(open)
        .unwrap_or_else(|| panic!("no `{open}` after `{from}`"))
        + open.len();
    let body = &rest[start..];
    let end = body
        .find(close)
        .unwrap_or_else(|| panic!("no `{close}` closing `{from}`"));
    &body[..end]
}

/// Every `"…"` or `'…'` literal of `source`, in order.
fn string_literals(source: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '"' && character != '\'' {
            continue;
        }
        let mut literal = String::new();
        for inner in characters.by_ref() {
            if inner == character {
                break;
            }
            literal.push(inner);
        }
        literals.push(literal);
    }
    literals
}

/// The prefix `ApiError::to_problem_details` puts in front of every slug.
fn rust_prefix() -> String {
    let declaration = between(RUST_SOURCE, "const PROBLEM_TYPE_PREFIX", "\"", "\"");
    assert!(
        !declaration.is_empty(),
        "PROBLEM_TYPE_PREFIX is declared empty"
    );
    declaration.to_owned()
}

/// The identifiers the API mints, read from the `slug` match arms themselves.
///
/// Read rather than listed: Rust cannot enumerate the variants of an enum, so a
/// hand-written list here would miss the seventh kind the day someone adds one —
/// which is precisely the drift this file exists to catch.
fn identifiers_the_api_mints() -> BTreeSet<String> {
    let prefix = rust_prefix();
    let body = between(RUST_SOURCE, "fn slug(", "{", "\n    }");
    let slugs = string_literals(body);
    assert!(
        slugs.len() >= 2,
        "`fn slug` yielded {} arm(s); the parser has lost the shape of it",
        slugs.len()
    );
    slugs
        .into_iter()
        .map(|slug| prefix.clone() + &slug)
        .collect()
}

/// The identifiers the front branches on, read from its `ProblemType` object.
fn identifiers_the_front_expects() -> BTreeSet<String> {
    let prefix = rust_prefix();
    let body = between(
        TYPESCRIPT_SOURCE,
        "export const ProblemType",
        "{",
        "} as const",
    );
    string_literals(body)
        .into_iter()
        .filter(|literal| literal.starts_with(&prefix))
        .collect()
}

#[test]
fn the_front_mirrors_exactly_the_problem_identifiers_the_api_mints() {
    // An equality between two sets, which is what makes it self-checking: an
    // identifier the front invented fails, a kind the backend added and the
    // front never learned fails, and a parser that stopped recognising either
    // shape fails too — it would leave one side short while the other still
    // reports its own.
    assert_eq!(
        identifiers_the_front_expects(),
        identifiers_the_api_mints(),
        "web/src/app/core/api/problem-details.ts and crates/api/src/error.rs \
         no longer agree on the `type` member"
    );
}

#[test]
fn the_front_carries_the_case_rfc_7807_reserves_for_no_problem_type() {
    // `about:blank` is not minted by the API: it is what the front puts on a
    // failure that never reached a handler (RFC 7807 §4.2). It therefore has to
    // be absent from the comparison above and present in the file, and the two
    // statements have to be checked together or the filter could quietly be
    // swallowing a real identifier.
    let body = between(
        TYPESCRIPT_SOURCE,
        "export const ProblemType",
        "{",
        "} as const",
    );
    let literals = string_literals(body);
    assert!(
        literals.iter().any(|literal| literal == "about:blank"),
        "the front no longer names the `about:blank` case: {literals:?}"
    );
    assert!(
        !identifiers_the_front_expects().contains("about:blank"),
        "`about:blank` may not be counted among the identifiers the API mints"
    );
}

#[test]
fn the_prefix_is_read_from_the_backend_and_not_assumed() {
    // Guards the reader itself. If `PROBLEM_TYPE_PREFIX` were ever parsed wrong
    // — an empty string, say — both sides of the comparison above would be built
    // with the same wrong prefix and it would still pass.
    let prefix = rust_prefix();
    assert!(
        prefix.starts_with('/'),
        "a relative reference is expected, got `{prefix}`"
    );
    let minted = identifiers_the_api_mints();
    assert!(
        minted.iter().all(|kind| kind.starts_with(&prefix)),
        "{minted:?}"
    );
    assert!(
        minted.iter().all(|kind| kind.len() > prefix.len()),
        "an identifier is nothing but the prefix: {minted:?}"
    );
}

// ---------------------------------------------------------------------------
// Pagination bounds
// ---------------------------------------------------------------------------

/// The module that mirrors the pagination bounds for the front.
const TYPESCRIPT_PAGE_SOURCE: &str = include_str!("../../../web/src/app/core/api/page.ts");

/// The value of `export const <name> = <integer>;` in the TypeScript.
fn typescript_integer(name: &str) -> u16 {
    let declaration = format!("export const {name} =");
    let value = between(TYPESCRIPT_PAGE_SOURCE, &declaration, "", ";").trim();
    value.parse().unwrap_or_else(|_| {
        panic!("`{name}` is not declared as a plain integer in page.ts: `{value}`")
    })
}

#[test]
fn the_front_pages_within_the_bounds_the_api_enforces() {
    // `page.ts` copies the two bounds of `crates/api/src/pagination.rs`, and
    // nothing tied the copy to the original: the verification of PR #46
    // changed them to 999 and 500 and the front lint and tests stayed green.
    // A front maximum above the API's sends a `limit` the API answers with a
    // 400. Compared with the constants the API validates with, not with their
    // source text, so that a reformatting on the Rust side cannot hide a drift.
    assert_eq!(
        typescript_integer("MAX_PAGE_LIMIT"),
        api::pagination::MAX_LIMIT,
        "web/src/app/core/api/page.ts and crates/api/src/pagination.rs \
         no longer agree on the largest page"
    );
    assert_eq!(
        typescript_integer("DEFAULT_PAGE_LIMIT"),
        api::pagination::DEFAULT_LIMIT,
        "web/src/app/core/api/page.ts and crates/api/src/pagination.rs \
         no longer agree on the default page"
    );
}

// ---------------------------------------------------------------------------
// Feeling suggestion
// ---------------------------------------------------------------------------

#[test]
fn every_rating_suggested_is_one_a_drink_accepts() {
    // The form of #29 pre-selects the `rating` of
    // `GET /profiles/{id}/feeling-suggestion` and posts it back as the
    // `feeling_rating` of the drink: a suggestion outside the scale the drink
    // endpoints validate would be answered with a 400 the user never chose.
    // Swept over the concentrations rather than read from the constants
    // alone, so that a scale overshooting its own maximum fails too.
    use api::drink::validation::{FEELING_RATING_MAX, FEELING_RATING_MIN};
    use domain::feeling::{SUGGESTED_RATING_MAX, SUGGESTED_RATING_MIN, suggested_rating};

    assert_eq!(i64::from(SUGGESTED_RATING_MIN), FEELING_RATING_MIN);
    assert!(i64::from(SUGGESTED_RATING_MAX) <= FEELING_RATING_MAX);
    for thousandths in 0..=10_000 {
        let rating = i64::from(suggested_rating(f64::from(thousandths) / 1000.0));
        assert!(
            (FEELING_RATING_MIN..=FEELING_RATING_MAX).contains(&rating),
            "{thousandths} mg/L suggests {rating}, which a drink refuses"
        );
    }
}

#[test]
fn a_feeling_suggestion_travels_under_the_members_the_front_reads() {
    // `{ at, bac_g_per_l, rating }`, the shape #73 sets for the form of #29,
    // checked on the serialised payload rather than on the struct, so that a
    // `serde` rename cannot slip past.
    let suggestion = api::bac::FeelingSuggestion {
        at: "2026-08-20T23:04:00Z".parse().expect("a real instant"),
        bac_g_per_l: 0.0,
        rating: 1,
    };
    let value = serde_json::to_value(&suggestion).expect("the payload must serialise");
    let members: BTreeSet<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        members,
        BTreeSet::from(["at", "bac_g_per_l", "rating"]),
        "{value}"
    );
    assert_eq!(value["at"], "2026-08-20T23:04:00Z");
}

// ---------------------------------------------------------------------------
// Event deletion
// ---------------------------------------------------------------------------

#[test]
fn a_drink_detached_from_its_event_travels_with_a_null_event_id() {
    // `DELETE /events/{id}` (#74) keeps the drinks entered from the event in
    // the history of their profile, detached from it. The screen of #23
    // reads that history afterwards: a detached drink carries
    // `"event_id": null`, the member present rather than left out, so that a
    // client comparing it with `null` does not meet `undefined`.
    let instant: api::Timestamp = "2026-08-20T21:00:00Z".parse().expect("a real instant");
    let drink = api::drink::Drink {
        id: api::new_id(),
        profile_id: api::new_id(),
        event_id: None,
        name: "Bière".to_owned(),
        ingested_at: instant,
        ingestion_duration_seconds: 1200,
        absorption_duration_seconds: 1800,
        quantity_unit: api::profile::QuantityUnit::Cl,
        volume_total_ml: None,
        feeling_rating: None,
        components: Vec::new(),
        created_at: instant,
        updated_at: instant,
    };
    let value = serde_json::to_value(&drink).expect("the payload must serialise");
    let members = value.as_object().expect("an object");
    assert!(members.contains_key("event_id"), "{value}");
    assert!(value["event_id"].is_null(), "{value}");
}

// ---------------------------------------------------------------------------
// Profile deletion
// ---------------------------------------------------------------------------

#[test]
fn a_deleted_profile_reads_as_the_not_found_problem_the_front_names() {
    // `DELETE /profiles/{id}` (#75) answers 204 without a body, and every
    // later read of the profile — `GET /profiles/{id}`, its drinks, its
    // curve — the not-found problem. The front keeps the profile it chose in
    // `localStorage` (`CurrentProfileStore`), so a profile deleted from
    // another device is met that way; the screen of #9 has to recognise it
    // by `type`, under the name `ProblemType.notFound`.
    let front = between(TYPESCRIPT_SOURCE, "notFound:", "'", "'");
    let back = api::ApiError::not_found("no profile with identifier …").to_problem_details();
    assert_eq!(back.status, 404);
    assert_eq!(
        back.kind, front,
        "web/src/app/core/api/problem-details.ts names another identifier \
         `notFound` than the one a deleted profile is read with"
    );
}
