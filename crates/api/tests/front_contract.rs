//! The one thing the front and the back have to spell identically.
//!
//! `web/src/app/core/api/problem-details.ts` mirrors the problem identifiers of
//! `crates/api/src/error.rs`. RFC 7807 makes `type` the stable member and the
//! front branches on it alone — `title` is English prose that may be reworded at
//! any time — so those six strings are the whole contract between the two sides.
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
//! seventh kind added on the backend. This file reads **both sources** and
//! compares them, so it sees both.
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
