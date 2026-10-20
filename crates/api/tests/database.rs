//! Integration tests that need a real PostgreSQL.
//!
//! There is no PostgreSQL service in CI (an acknowledged gap, tracked outside
//! this issue), so every test here is conditioned on `DATABASE_URL` and skips
//! itself when it is not set — `cargo test --workspace` therefore stays green on
//! a machine without a database. They are not marked `#[ignore]`: an ignored
//! test never runs, not even where a database *is* available.
//!
//! **A skipped test passes silently.** `libtest` captures the output of a test
//! that passes, so the `skipping …` line printed below is invisible under the
//! command the CI gate runs: a run with a database and a run without one report
//! the same `N passed`. Two things make the difference visible on purpose:
//!
//! ```text
//! # See which tests skipped and why:
//! cargo test -p api --test database -- --nocapture
//!
//! # Fail instead of skipping — this is how you check the suite still bites,
//! # e.g. after renaming an environment variable or touching `Config`:
//! ALCOLOCO_REQUIRE_DB=1 DATABASE_URL=… cargo test --workspace
//! ```
//!
//! Replay them against a database with:
//!
//! ```text
//! DATABASE_URL=postgres://alcoloco:alcoloco@localhost:5432/alcoloco \
//!   cargo test --workspace
//! ```

use std::collections::BTreeSet;

use api::config::{DATABASE_URL_ENV, ENVIRONMENT_ENV};
use api::profile::validation::{
    ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS, HEIGHT_CM_MAX_EXCLUSIVE, HEIGHT_CM_MIN_EXCLUSIVE,
    INGESTION_DURATION_MIN_SECONDS, WEIGHT_KG_MAX_EXCLUSIVE, WEIGHT_KG_MIN_EXCLUSIVE,
};
use api::{AppState, Config};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

/// Set to a non-empty value other than `0`, turns every skip into a failure.
const REQUIRE_DATABASE_ENV: &str = "ALCOLOCO_REQUIRE_DB";

/// Whether skipping is forbidden for this run.
fn skipping_is_forbidden() -> bool {
    std::env::var(REQUIRE_DATABASE_ENV)
        .is_ok_and(|value| !value.trim().is_empty() && value.trim() != "0")
}

/// The connection string to test against, or `None` when there is none.
///
/// The printed line only reaches a terminal under `-- --nocapture`; the switch
/// that makes a skip *impossible to miss* is [`REQUIRE_DATABASE_ENV`], which
/// turns the skip into a failure. Without it, a run on a machine with no
/// database is indistinguishable from a run against one — which is exactly why
/// the switch exists.
fn database_url(test: &str) -> Option<String> {
    match std::env::var(DATABASE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            assert!(
                !skipping_is_forbidden(),
                "`{test}` may not skip: {REQUIRE_DATABASE_ENV} is set but \
                 {DATABASE_URL_ENV} is not"
            );
            eprintln!(
                "skipping `{test}`: {DATABASE_URL_ENV} is not set \
                 (only visible under `-- --nocapture`)"
            );
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
    let answer = send(state, "GET", uri, None).await;
    (answer.status, answer.body)
}

/// What one call to the router gave back.
struct Answer {
    status: StatusCode,
    media_type: String,
    location: Option<String>,
    body: Value,
}

/// Drives the real router in process, body and all.
async fn send(state: AppState, method: &str, uri: &str, payload: Option<Value>) -> Answer {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match payload {
        None => Body::empty(),
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
    };
    let response = api::app(state)
        .oneshot(builder.body(body).expect("the request must build"))
        .await
        .expect("the router must answer");

    let status = response.status();
    let text = |name: header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let media_type = text(header::CONTENT_TYPE).unwrap_or_default();
    let location = text(header::LOCATION);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body must be readable")
        .to_bytes();

    Answer {
        status,
        media_type,
        location,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// A database this test may write to, with every migration applied.
///
/// `db::migrate` is idempotent and takes an advisory lock, so several tests of
/// this file may call it at once. The database is **not** reset: `db::reset`
/// drops the whole `public` schema, which would pull the tables out from under
/// every test running in parallel. Each test therefore creates its own profiles
/// and asserts on those only.
async fn migrated(test: &str) -> Option<AppState> {
    let state = state(&database_url(test)?);
    db::migrate(&state.pool)
        .await
        .expect("the migrations must apply");
    Some(state)
}

/// A profile creation payload that satisfies every bound.
fn a_sound_payload(display_name: &str) -> Value {
    serde_json::json!({
        "display_name": display_name,
        "default_quantity_unit": "cl",
        "default_ingestion_duration_seconds": 1200,
        "default_absorption_duration_seconds": 1800,
        "settings": {
            "weight_kg": 62.0,
            "height_cm": 168.0,
            "sex": "female",
            "birth_date": "1994-05-12",
        },
    })
}

/// Creates a profile through the API and answers its identifier and body.
async fn create_profile(state: &AppState, payload: &Value) -> (Uuid, Value) {
    let answer = send(state.clone(), "POST", "/profiles", Some(payload.clone())).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "creation refused: {}",
        answer.body
    );
    let id = answer.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no identifier in {}", answer.body))
        .parse()
        .expect("the identifier must be a UUID");
    assert_eq!(
        answer.location.as_deref(),
        Some(&*format!("/profiles/{id}"))
    );
    (id, answer.body)
}

/// How many settings versions a profile owns.
async fn version_count(pool: &sqlx::PgPool, profile: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM public.profile_settings_version WHERE profile_id = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}

/// `sql` lowercased, with its `--` and `/* */` comments removed (block comments
/// nest, as in PostgreSQL) and every run of whitespace collapsed to one space.
///
/// Single-quoted strings and double-quoted identifiers are copied as they are,
/// so that a `--` or a `/*` inside a literal is not taken for a comment. Dollar
/// quotes are not recognised: a function body is scanned like the rest of the
/// file, which is right for its comments and its own quoted strings. What this
/// does not handle is an `E'…\'…'` string, whose backslash-escaped quote would
/// end the literal early; no migration writes one.
fn without_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '-' if chars.peek() == Some(&'-') => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                }
                out.push(' ');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut depth = 1;
                while depth > 0 {
                    match chars.next() {
                        Some('/') if chars.peek() == Some(&'*') => {
                            chars.next();
                            depth += 1;
                        }
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            depth -= 1;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                out.push(' ');
            }
            '\'' | '"' => {
                // A doubled quote inside a literal closes it and reopens it at
                // once, which copies the same characters.
                out.push(character);
                for inner in chars.by_ref() {
                    out.push(inner);
                    if inner == character {
                        break;
                    }
                }
            }
            _ => out.push(character),
        }
    }

    out.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Names a `CREATE TABLE` / `DROP TABLE` statement of `sql` acts on.
///
/// `keyword` is matched on [`without_comments`], so it must be given lowercase,
/// with single spaces, and end with a space. A comma-separated list of names —
/// `DROP TABLE a, b` — yields every name of the list. Any schema qualification
/// is dropped: only the bare name is returned, which is what
/// `information_schema` reports.
fn tables_named_after(sql: &str, keyword: &str) -> Vec<String> {
    let normalised = without_comments(sql);
    let mut names = Vec::new();
    let mut from = 0;

    while let Some(at) = normalised[from..].find(keyword) {
        let after = from + at + keyword.len();
        let mut rest = normalised[after..].trim_start();
        for guard in ["if not exists ", "if exists "] {
            if let Some(stripped) = rest.strip_prefix(guard) {
                rest = stripped.trim_start();
            }
        }
        loop {
            let qualified: String = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || *character == '_' || *character == '.'
                })
                .collect();
            if let Some(name) = qualified.rsplit('.').next().filter(|name| !name.is_empty()) {
                names.push(name.to_owned());
            }
            match rest[qualified.len()..].trim_start().strip_prefix(',') {
                Some(next) if !qualified.is_empty() => rest = next.trim_start(),
                _ => break,
            }
        }
        from = after;
    }

    names
}

/// The tables the migrations of the `db` crate declare, read from the migration
/// SQL itself rather than copied out of it by hand — a list transcribed here
/// would drift away from the schema at the first migration of #10, #13 or #43,
/// and drift silently.
fn tables_the_migrations_declare() -> BTreeSet<String> {
    tables_declared_by(db::MIGRATOR.migrations.iter().map(|m| m.sql.as_ref()))
}

/// The tables left declared once every script of `scripts` has run, in order.
fn tables_declared_by<'a>(scripts: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    for sql in scripts {
        for name in tables_named_after(sql, "create table ") {
            declared.insert(name);
        }
        for name in tables_named_after(sql, "drop table ") {
            declared.remove(&name);
        }
    }
    declared
}

/// The set of `names`, for the assertions on the scan below.
fn table_set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

// The scan above is what the schema test compares the database against, so a
// mistake in it shows up there as a disagreement on the tables. These cases pin
// the shapes a correct migration may legitimately take. Four of them — a table
// named in a comment, keywords split by other whitespace than one space, a
// `DROP TABLE` list, a table named in a comment after an apostrophe — were
// read wrong by the previous scan, which made the schema test fail on a schema
// that was right. The fifth, comment markers inside a string, passed on it; it
// guards the string handling added since.

#[test]
fn the_scan_ignores_a_table_named_in_a_comment() {
    let sql = "-- a reader could CREATE TABLE ghost (a int) here
               /* or CREATE TABLE phantom (a int) there */
               /* nested /* CREATE TABLE inner_ghost */ CREATE TABLE outer_ghost */
               CREATE TABLE real_one (a int);";
    assert_eq!(tables_declared_by([sql]), table_set(&["real_one"]));
}

#[test]
fn the_scan_reads_keywords_separated_by_any_whitespace() {
    let sql = "CREATE\n    TABLE spread_out (a int);
               CREATE  TABLE\tIF  NOT\n EXISTS guarded(a int);
               CREATE TABLE -- the name comes next
                   commented_between (a int);";
    assert_eq!(
        tables_declared_by([sql]),
        table_set(&["commented_between", "guarded", "spread_out"])
    );
}

#[test]
fn the_scan_drops_every_table_of_a_drop_list() {
    let create = "CREATE TABLE a (x int); CREATE TABLE b (x int); CREATE TABLE c (x int);";
    let drop = "DROP TABLE a, public.b;";
    assert_eq!(tables_declared_by([create, drop]), table_set(&["c"]));
}

#[test]
fn the_scan_does_not_take_comment_markers_in_a_string_for_comments() {
    let sql = "INSERT INTO t VALUES ('Jus d''orange -- 0 %'); CREATE TABLE kept (a int);
               INSERT INTO t VALUES ('/* not a comment'); CREATE TABLE also_kept (a int);";
    assert_eq!(tables_declared_by([sql]), table_set(&["also_kept", "kept"]));
}

#[test]
fn the_scan_does_not_take_an_apostrophe_in_a_comment_for_a_string() {
    // A single apostrophe in the whole script, so that no second one can close
    // a string wrongly opened by the first. Ending the comment at it would
    // either read the rest of the line as code (and declare `ghost`) or open a
    // string running to the end of the script (and lose `after_the_comment`).
    let sql = "-- the profile's CREATE TABLE ghost (a int) -- CREATE TABLE ghost_too (a int)
               CREATE TABLE after_the_comment (a int);";
    assert_eq!(tables_declared_by([sql]), table_set(&["after_the_comment"]));
}

/// The tables actually present in `public`, minus sqlx's own bookkeeping table.
async fn tables_in_the_database(pool: &sqlx::PgPool) -> BTreeSet<String> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
           AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(pool)
    .await
    .expect("the schema query must run");
    names.into_iter().collect()
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
    // leave behind holds exactly the tables the migration SQL declares. No Rust
    // gate touches the database, so nothing else in the workspace would notice
    // a migration that no longer applies.
    //
    // The comparison is an equality between two sets, and that is what makes it
    // self-checking: a table missing from the database fails, a table nobody
    // declared fails, and a parser that stopped recognising `CREATE TABLE` also
    // fails — it would leave the declared side short while the database still
    // reports its tables.
    let name = "the_pool_the_api_builds_reaches_the_migrated_schema";
    let Some(url) = database_url(name) else {
        return;
    };

    let state = state(&url);
    db::migrate(&state.pool)
        .await
        .expect("the migrations must apply");

    assert_eq!(
        tables_in_the_database(&state.pool).await,
        tables_the_migrations_declare(),
        "the migrated schema and the migration SQL disagree on the tables"
    );
}

// ---------------------------------------------------------------------------
// Profiles (issue #6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_profile_table_carries_no_copy_of_the_versioned_parameters() {
    // The arbitration of 2026-08-19 (SPEC.md §4, §5.1, §10.0-L), checked against
    // the real catalogue rather than against the migration text: weight, height,
    // sex and birth date live in `profile_settings_version` and nowhere else. A
    // current copy on `profile` would be a second source of truth, free to
    // diverge at the first update and read by no computation.
    let name = "the_profile_table_carries_no_copy_of_the_versioned_parameters";
    let Some(state) = migrated(name).await else {
        return;
    };

    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1",
    )
    .bind("profile")
    .fetch_all(&state.pool)
    .await
    .expect("the catalogue query must run");

    for versioned in ["weight_kg", "height_cm", "sex", "birth_date", "age", "beta"] {
        assert!(
            !columns.contains(&versioned.to_owned()),
            "`profile` carries `{versioned}`, which belongs to the version: {columns:?}"
        );
    }
    // Guards the query itself: a typo in the table name would return no column
    // at all and let the loop above pass on an empty list.
    assert!(columns.contains(&"display_name".to_owned()), "{columns:?}");
}

#[tokio::test]
async fn creating_a_profile_writes_its_parameters_as_a_first_settings_version() {
    let name = "creating_a_profile_writes_its_parameters_as_a_first_settings_version";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, body) = create_profile(&state, &a_sound_payload("Création")).await;

    assert_eq!(body["display_name"], "Création");
    assert_eq!(body["settings"]["weight_kg"], 62.0);
    assert_eq!(body["settings"]["sex"], "female");
    assert_eq!(body["settings"]["birth_date"], "1994-05-12");
    assert!(
        body["settings"]["valid_from"]
            .as_str()
            .is_some_and(|at| at.ends_with('Z')),
        "valid_from must be ISO 8601 UTC: {body}"
    );
    assert_eq!(
        version_count(&state.pool, id).await,
        1,
        "a creation posts exactly one version"
    );

    // And the profile is readable afterwards under the path the `Location`
    // header announced.
    let read = send(state, "GET", &format!("/profiles/{id}"), None).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body, body);
}

#[tokio::test]
async fn a_profile_without_a_sex_or_a_birth_date_is_refused() {
    // Acceptance criterion of issue #6. What is checked is not only the refusal
    // but *how* it is reported: a problem document naming both missing members,
    // so a client learns everything it left out in one answer.
    let name = "a_profile_without_a_sex_or_a_birth_date_is_refused";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Sans paramètres");
    payload["settings"]["sex"] = Value::Null;
    payload["settings"]["birth_date"] = Value::Null;

    let answer = send(state.clone(), "POST", "/profiles", Some(payload)).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.media_type, "application/problem+json");
    assert_eq!(answer.body["type"], "/problems/validation_failed");

    let offending: Vec<&str> = answer.body["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("no errors[] in {}", answer.body))
        .iter()
        .filter_map(|error| error["field"].as_str())
        .collect();
    assert_eq!(offending, vec!["settings.sex", "settings.birth_date"]);

    // Nothing was written: the profile does not exist under any name.
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.profile WHERE display_name = $1")
            .bind("Sans paramètres")
            .fetch_one(&state.pool)
            .await
            .expect("the count must run");
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn changing_only_the_input_preferences_posts_no_version() {
    // SPEC.md §10.0-J: the input preferences enter no computation, so they are
    // not versioned. Posting a version for them would grow the history of every
    // profile for changes no curve can ever see.
    let name = "changing_only_the_input_preferences_posts_no_version";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, created) = create_profile(&state, &a_sound_payload("Préférences")).await;

    let mut payload = a_sound_payload("Préférences renommées");
    payload["default_quantity_unit"] = Value::from("percent");
    payload["default_ingestion_duration_seconds"] = Value::from(600);
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["display_name"], "Préférences renommées");
    assert_eq!(answer.body["default_quantity_unit"], "percent");
    assert_eq!(answer.body["default_ingestion_duration_seconds"], 600);
    assert_eq!(
        answer.body["settings"], created["settings"],
        "the parameters in force may not move when only preferences change"
    );
    assert_eq!(version_count(&state.pool, id).await, 1);
}

#[tokio::test]
async fn changing_a_physiological_parameter_posts_a_version() {
    // The other half of the same criterion: "a change of the physiological
    // parameters triggers the creation of a version" (issue #6).
    let name = "changing_a_physiological_parameter_posts_a_version";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, created) = create_profile(&state, &a_sound_payload("Poids")).await;

    let mut payload = a_sound_payload("Poids");
    payload["settings"]["weight_kg"] = Value::from(64.5);
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings"]["weight_kg"], 64.5);
    assert_ne!(
        answer.body["settings"]["valid_from"], created["settings"]["valid_from"],
        "the new version must carry its own effective date"
    );
    assert_eq!(version_count(&state.pool, id).await, 2);
}

/// Writes one settings version straight to the database, at `valid_from`, with
/// the parameters of [`a_sound_payload`] except the weight.
async fn insert_version_at(pool: &sqlx::PgPool, profile: Uuid, valid_from: &str, weight_kg: f64) {
    sqlx::query(
        "INSERT INTO public.profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, $3::timestamptz, $4, 168.0, 'female'::public.sex, '1994-05-12'::date)",
    )
    .bind(api::new_id())
    .bind(profile)
    .bind(valid_from)
    .bind(weight_kg)
    .execute(pool)
    .await
    .expect("the version must be written");
}

/// The settings history of a profile, oldest first, as `(valid_from, weight)`.
async fn history(pool: &sqlx::PgPool, profile: Uuid) -> Vec<(String, f64)> {
    sqlx::query_as(
        "SELECT to_char(valid_from AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), weight_kg
         FROM public.profile_settings_version
         WHERE profile_id = $1
         ORDER BY valid_from",
    )
    .bind(profile)
    .fetch_all(pool)
    .await
    .expect("the history must be readable")
}

/// The weight in force for a profile at `at`, through `profile_settings_at()`.
async fn weight_at(pool: &sqlx::PgPool, profile: Uuid, at: &str) -> f64 {
    sqlx::query_scalar("SELECT weight_kg FROM public.profile_settings_at($1, $2::timestamptz)")
        .bind(profile)
        .bind(at)
        .fetch_one(pool)
        .await
        .expect("the lookup must run")
}

#[tokio::test]
async fn a_retroactive_valid_from_replaces_the_later_versions_and_says_so() {
    // The "I had the wrong weight" gesture of SPEC.md §5.1: posting a version
    // at T closes the previous one at T and **replaces** every version already
    // later than T, and the answer says how many were replaced (issue #7).
    //
    // The history is built so that every branch of that rule is exercised at
    // once: one version strictly before T that must survive and close at T, and
    // three strictly after it — two written straight to the database, plus the
    // one the creation posted at now — that must all go.
    let name = "a_retroactive_valid_from_replaces_the_later_versions_and_says_so";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Rétroactif")).await;
    for (at, weight) in [
        ("2020-01-01T00:00:00Z", 50.0),
        ("2022-01-01T00:00:00Z", 52.0),
        ("2023-01-01T00:00:00Z", 53.0),
    ] {
        insert_version_at(&state.pool, id, at, weight).await;
    }
    assert_eq!(version_count(&state.pool, id).await, 4);

    let corrected_from = "2021-06-01T00:00:00Z";
    let mut payload = a_sound_payload("Rétroactif");
    payload["settings"]["weight_kg"] = Value::from(58.0);
    payload["settings"]["valid_from"] = Value::from(corrected_from);
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_version_created"], true);
    assert_eq!(answer.body["settings_versions_replaced"], 3);
    assert_eq!(
        answer.body["settings"]["weight_kg"], 58.0,
        "nothing later than the correction is left, so it is the version in force now"
    );
    assert_eq!(answer.body["settings"]["valid_from"], corrected_from);

    assert_eq!(
        history(&state.pool, id).await,
        vec![
            ("2020-01-01T00:00:00Z".to_owned(), 50.0),
            (corrected_from.to_owned(), 58.0),
        ],
        "the version before T is kept and closes at T; the ones after T are gone"
    );
    assert_eq!(
        weight_at(&state.pool, id, "2021-05-31T23:59:59Z").await,
        50.0
    );
    assert_eq!(weight_at(&state.pool, id, corrected_from).await, 58.0);
}

#[tokio::test]
async fn each_retroactive_correction_answers_the_weight_it_posted() {
    // Replays the sequence the verification of PR #50 measured: a creation at
    // 88.5 kg, then two retroactive corrections, each earlier than the last.
    // Under #6 the second one answered 200 with the *old* weight and no signal.
    let name = "each_retroactive_correction_answers_the_weight_it_posted";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Vérification #6");
    payload["settings"]["weight_kg"] = Value::from(88.5);
    let (id, _) = create_profile(&state, &payload).await;

    for (valid_from, weight) in [
        ("2025-06-01T00:00:00Z", 91.0),
        ("2024-03-15T00:00:00Z", 95.25),
    ] {
        payload["settings"]["weight_kg"] = Value::from(weight);
        payload["settings"]["valid_from"] = Value::from(valid_from);
        let answer = send(
            state.clone(),
            "PUT",
            &format!("/profiles/{id}"),
            Some(payload.clone()),
        )
        .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        assert_eq!(answer.body["settings_version_created"], true);
        assert_eq!(answer.body["settings_versions_replaced"], 1);
        assert_eq!(answer.body["settings"]["weight_kg"], weight);
    }
    assert_eq!(
        history(&state.pool, id).await,
        vec![("2024-03-15T00:00:00Z".to_owned(), 95.25)]
    );
}

#[tokio::test]
async fn repeating_the_values_in_force_at_t_still_replaces_what_follows_t() {
    // A request says "from T on, these are the values". When they already are
    // the values in force at T, no version needs posting — but the versions
    // after T contradict the request and are replaced all the same; otherwise
    // the call would answer 200 and leave the profile saying something else.
    let name = "repeating_the_values_in_force_at_t_still_replaces_what_follows_t";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Répétition");
    payload["settings"]["weight_kg"] = Value::from(70.0);
    let (id, _) = create_profile(&state, &payload).await;
    insert_version_at(&state.pool, id, "2020-01-01T00:00:00Z", 62.0).await;

    payload["settings"]["weight_kg"] = Value::from(62.0);
    payload["settings"]["valid_from"] = Value::from("2021-01-01T00:00:00Z");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_version_created"], false);
    assert_eq!(answer.body["settings_versions_replaced"], 1);
    assert_eq!(answer.body["settings"]["weight_kg"], 62.0);
    assert_eq!(
        history(&state.pool, id).await,
        vec![("2020-01-01T00:00:00Z".to_owned(), 62.0)]
    );
}

#[tokio::test]
async fn a_correction_earlier_than_the_whole_history_keeps_one_version() {
    // T before every version: all of them are later than T, the oldest one
    // included — which is also the one `profile_settings_at(id, T)` falls back
    // to through the open lower bound. Comparing against that fallback and then
    // deleting it would leave the profile empty and the COMMIT refused.
    let name = "a_correction_earlier_than_the_whole_history_keeps_one_version";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Avant tout")).await;

    let mut payload = a_sound_payload("Avant tout");
    payload["settings"]["valid_from"] = Value::from("2001-01-01T00:00:00Z");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_version_created"], true);
    assert_eq!(answer.body["settings_versions_replaced"], 1);
    assert_eq!(
        history(&state.pool, id).await,
        vec![("2001-01-01T00:00:00Z".to_owned(), 62.0)]
    );
}

#[tokio::test]
async fn a_put_says_whether_it_posted_a_version() {
    // The signal the verification of #6 found missing. A write of the input
    // preferences alone posts nothing and replaces nothing; a change of weight
    // at now posts one version and, no version being allowed to start in the
    // future, replaces none.
    let name = "a_put_says_whether_it_posted_a_version";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Signal")).await;

    let mut payload = a_sound_payload("Signal renommé");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload.clone()),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_version_created"], false);
    assert_eq!(answer.body["settings_versions_replaced"], 0);

    payload["settings"]["weight_kg"] = Value::from(63.0);
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_version_created"], true);
    assert_eq!(answer.body["settings_versions_replaced"], 0);
    assert_eq!(version_count(&state.pool, id).await, 2);
}

#[tokio::test]
async fn a_refused_put_replaces_nothing() {
    // The replacement and the post are one transaction: a post refused for its
    // duplicate `valid_from` must leave the versions it would have replaced.
    let name = "a_refused_put_replaces_nothing";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Refus")).await;
    insert_version_at(&state.pool, id, "2020-01-01T00:00:00Z", 50.0).await;
    let before = history(&state.pool, id).await;

    let mut payload = a_sound_payload("Refus");
    payload["settings"]["weight_kg"] = Value::from(51.0);
    payload["settings"]["valid_from"] = Value::from("2020-01-01T00:00:00Z");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(history(&state.pool, id).await, before);
}

/// A drink as the acceptance test of issue #7 needs it: when, and how much
/// alcohol. The engine of #16 will read far more; this is enough to tell which
/// settings version a drink was computed with.
struct Drink {
    ingested_at: &'static str,
    volume_ml: f64,
    abv_percent: f64,
}

/// Blood alcohol a single drink produces once fully absorbed, before any
/// elimination, computed with the settings in force at its ingestion time and
/// the age at that same instant (SPEC.md §5.6, §10.0-K).
async fn bac_of(pool: &sqlx::PgPool, profile: Uuid, drink: &Drink) -> f64 {
    let at: api::Timestamp = drink.ingested_at.parse().expect("a real instant");
    let settings = api::profile::store::settings_at(pool, profile, at)
        .await
        .expect("the lookup must run")
        .expect("a profile always has an applicable version");
    let age_days = (at.date_naive() - settings.birth_date).num_days();
    let age_years = f64::from(i32::try_from(age_days).expect("an age in days fits")) / 365.2425;
    let sex = match settings.sex {
        api::profile::Sex::Male => domain::Sex::Male,
        api::profile::Sex::Female => domain::Sex::Female,
    };
    let water = domain::watson_total_body_water_litres(
        sex,
        settings.weight_kg,
        settings.height_cm,
        age_years,
    );
    domain::initial_bac_g_per_l(
        domain::ingested_alcohol_grams(drink.volume_ml, drink.abv_percent),
        water,
    )
}

#[tokio::test]
async fn a_weight_change_moves_only_the_drinks_after_it_and_a_correction_moves_its_period() {
    // The two acceptance criteria of issue #7, which have to hold together:
    //
    // - "I gained weight" (valid_from between the two drinks): the earlier
    //   drink keeps exactly the alcohol level it had, the later one moves;
    // - "I had the wrong weight" (valid_from earlier than both): the period the
    //   correction covers is recomputed with the corrected value.
    //
    // Male, so that the age term of Watson is in play and is taken at each
    // drink's own ingestion time.
    let name = "a_weight_change_moves_only_the_drinks_after_it_and_a_correction_moves_its_period";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Deux verres");
    payload["settings"]["sex"] = Value::from("male");
    payload["settings"]["weight_kg"] = Value::from(80.0);
    payload["settings"]["height_cm"] = Value::from(180.0);
    payload["settings"]["valid_from"] = Value::from("2025-01-01T00:00:00Z");
    let (id, _) = create_profile(&state, &payload).await;

    let before = Drink {
        ingested_at: "2025-03-01T21:00:00Z",
        volume_ml: 250.0,
        abv_percent: 5.0,
    };
    let after = Drink {
        ingested_at: "2025-09-01T21:00:00Z",
        volume_ml: 250.0,
        abv_percent: 5.0,
    };
    let before_at_first = bac_of(&state.pool, id, &before).await;
    let after_at_first = bac_of(&state.pool, id, &after).await;

    // "I gained weight", effective between the two drinks.
    payload["settings"]["weight_kg"] = Value::from(95.0);
    payload["settings"]["valid_from"] = Value::from("2025-06-01T00:00:00Z");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload.clone()),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_versions_replaced"], 0);

    assert_eq!(
        bac_of(&state.pool, id, &before).await.to_bits(),
        before_at_first.to_bits(),
        "a weight change may not move a drink ingested before it"
    );
    let after_heavier = bac_of(&state.pool, id, &after).await;
    assert!(
        after_heavier < after_at_first - 1e-6,
        "a heavier body dilutes the later drink: {after_heavier} vs {after_at_first}"
    );

    // "I had the wrong weight": corrected from before both drinks. The version
    // posted above starts after that instant, so it is replaced.
    payload["settings"]["weight_kg"] = Value::from(70.0);
    payload["settings"]["valid_from"] = Value::from("2025-02-01T00:00:00Z");
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["settings_versions_replaced"], 1);

    let before_corrected = bac_of(&state.pool, id, &before).await;
    let after_corrected = bac_of(&state.pool, id, &after).await;
    assert!(
        before_corrected > before_at_first + 1e-6,
        "the correction covers the earlier drink and must move it: \
         {before_corrected} vs {before_at_first}"
    );
    assert!(
        after_corrected > after_heavier + 1e-6 && after_corrected > after_at_first + 1e-6,
        "the correction covers the later drink too: {after_corrected}"
    );
}

/// Writes one settings version with the given birth date straight to the
/// database, inside the caller's transaction, from a session set to
/// `time_zone`, and says whether the schema accepted it.
///
/// The zone is set with `SET LOCAL` semantics, so it never outlives the
/// transaction. The write runs under a savepoint: a refused write is rolled
/// back to it, and the transaction stays usable for the next one.
async fn schema_accepts_birth_date(
    tx: &mut sqlx::PgConnection,
    profile: Uuid,
    birth_date: &str,
    time_zone: &str,
) -> Result<(), String> {
    sqlx::query("SELECT pg_catalog.set_config('TimeZone', $1, true)")
        .bind(time_zone)
        .execute(&mut *tx)
        .await
        .expect("the time zone must be accepted");
    sqlx::query("SAVEPOINT birth_date_write")
        .execute(&mut *tx)
        .await
        .expect("the savepoint must be set");
    let written = sqlx::query(
        "INSERT INTO public.profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, now() - ($3 || ' seconds')::interval, 62.0, 168.0,
                   'female'::public.sex, $4::date)",
    )
    .bind(api::new_id())
    .bind(profile)
    .bind(fastrand_seconds())
    .bind(birth_date)
    .execute(&mut *tx)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string());
    let closing = if written.is_ok() {
        "RELEASE SAVEPOINT birth_date_write"
    } else {
        "ROLLBACK TO SAVEPOINT birth_date_write"
    };
    sqlx::query(closing)
        .execute(&mut *tx)
        .await
        .expect("the savepoint must close");
    written
}

#[tokio::test]
async fn the_schema_refuses_a_birth_date_in_the_future() {
    // The API refuses it already; this is the database refusing it too, for
    // every writer that does not go through the API. Today **in UTC** is the
    // last date accepted, as it is for the API, whatever zone the writing
    // session is set to.
    //
    // Two zones on either side of UTC, because one of them alone does not
    // always tell UTC apart from the session zone: Kiritimati (UTC+14) is
    // already on tomorrow's UTC date from 10:00 UTC on, Pago Pago (UTC-11) is
    // still on yesterday's until 11:00 UTC, so at any instant at least one of
    // the two dates differs from the UTC one. A rule read in the session zone
    // then either accepts tomorrow (UTC) or refuses today (UTC) there.
    //
    // One transaction for the reading of today and for every write, because
    // `now()` is frozen at the start of a transaction: the trigger then judges
    // every write against the very date the test read, even when midnight UTC
    // passes while the test runs. Across separate transactions, today would
    // turn into yesterday and tomorrow into today at midnight, and the test
    // would fail with nothing wrong in the schema. Each refusal is rolled back
    // to a savepoint, so that the transaction goes on.
    let name = "the_schema_refuses_a_birth_date_in_the_future";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Naissance")).await;
    let mut tx = state.pool.begin().await.expect("a transaction must open");
    let (today, tomorrow): (String, String) = sqlx::query_as(
        "SELECT ((now() AT TIME ZONE 'UTC')::date)::pg_catalog.text,
                ((now() AT TIME ZONE 'UTC')::date + 1)::pg_catalog.text",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("the dates must be readable");

    for zone in ["UTC", "Pacific/Kiritimati", "Pacific/Pago_Pago"] {
        schema_accepts_birth_date(&mut tx, id, &today, zone)
            .await
            .unwrap_or_else(|error| panic!("today (UTC) is not in the future in {zone}: {error}"));
        let refused = schema_accepts_birth_date(&mut tx, id, &tomorrow, zone)
            .await
            .expect_err("a birth date after today (UTC) must be refused");
        assert!(refused.contains("birth date"), "{zone}: {refused}");
    }

    sqlx::query("SAVEPOINT birth_date_update")
        .execute(&mut *tx)
        .await
        .expect("the savepoint must be set");
    let moved = sqlx::query(
        "UPDATE public.profile_settings_version SET birth_date = $2::date WHERE profile_id = $1",
    )
    .bind(id)
    .bind(&tomorrow)
    .execute(&mut *tx)
    .await
    .expect_err("an UPDATE may not move a birth date into the future either")
    .to_string();
    sqlx::query("ROLLBACK TO SAVEPOINT birth_date_update")
        .execute(&mut *tx)
        .await
        .expect("the refused UPDATE must roll back to its savepoint");
    assert!(moved.contains("birth date"), "{moved}");
}

#[tokio::test]
async fn two_versions_may_not_start_at_the_same_instant() {
    // The one unique constraint of the version table, reported as the caller's
    // fault rather than as a 500 carrying a constraint name.
    let name = "two_versions_may_not_start_at_the_same_instant";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, created) = create_profile(&state, &a_sound_payload("Doublon")).await;

    let mut payload = a_sound_payload("Doublon");
    payload["settings"]["weight_kg"] = Value::from(70.0);
    payload["settings"]["valid_from"] = created["settings"]["valid_from"].clone();
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{id}"),
        Some(payload),
    )
    .await;

    assert_eq!(answer.status, StatusCode::CONFLICT, "{}", answer.body);
    assert_eq!(answer.media_type, "application/problem+json");
    assert_eq!(answer.body["type"], "/problems/conflict");
    assert_eq!(
        version_count(&state.pool, id).await,
        1,
        "the refused write must leave no version behind"
    );
}

#[tokio::test]
async fn the_collection_is_walked_by_cursor_and_never_serves_a_row_twice() {
    // The pagination contract, exercised end to end against a real table. Other
    // tests of this file write concurrently, so the walk collects everything and
    // then looks for the profiles this test created: what is asserted is that
    // each appears exactly once and in identifier order, which is the property
    // an `OFFSET` would break.
    let name = "the_collection_is_walked_by_cursor_and_never_serves_a_row_twice";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut mine = Vec::new();
    for index in 0..3 {
        let (id, _) = create_profile(&state, &a_sound_payload(&format!("Page {index}"))).await;
        mine.push(id);
    }

    let mut seen: Vec<Uuid> = Vec::new();
    let mut uri = "/profiles?limit=2".to_owned();
    for _ in 0..1000 {
        let answer = send(state.clone(), "GET", &uri, None).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        let items = answer.body["items"]
            .as_array()
            .unwrap_or_else(|| panic!("no items[] in {}", answer.body));
        assert!(
            items.len() <= 2,
            "a page larger than the limit: {}",
            answer.body
        );
        for item in items {
            seen.push(
                item["id"]
                    .as_str()
                    .expect("an item must carry an identifier")
                    .parse()
                    .expect("an identifier must be a UUID"),
            );
        }
        match answer.body["next_cursor"].as_str() {
            None => break,
            Some(cursor) => uri = format!("/profiles?limit=2&cursor={cursor}"),
        }
    }

    let found: Vec<Uuid> = seen
        .iter()
        .copied()
        .filter(|id| mine.contains(id))
        .collect();
    assert_eq!(found, mine, "walked: {seen:?}");
    assert_eq!(
        {
            let mut unique = seen.clone();
            unique.sort_unstable();
            unique.dedup();
            unique.len()
        },
        seen.len(),
        "a row was served twice: {seen:?}"
    );
}

#[tokio::test]
async fn a_page_size_beyond_the_cap_is_reported_rather_than_served() {
    let name = "a_page_size_beyond_the_cap_is_reported_rather_than_served";
    let Some(state) = migrated(name).await else {
        return;
    };

    let answer = send(state, "GET", "/profiles?limit=201", None).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.media_type, "application/problem+json");
    assert_eq!(answer.body["errors"][0]["field"], "limit");
}

#[tokio::test]
async fn an_unknown_profile_is_a_problem_document_and_an_unusable_one_is_another() {
    let name = "an_unknown_profile_is_a_problem_document_and_an_unusable_one_is_another";
    let Some(state) = migrated(name).await else {
        return;
    };

    let missing = api::new_id();
    let answer = send(state.clone(), "GET", &format!("/profiles/{missing}"), None).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.media_type, "application/problem+json");
    assert_eq!(answer.body["type"], "/problems/not_found");

    // An identifier that is not a UUID is a malformed request, not a missing
    // resource: 404 would say the value could have named something.
    let answer = send(state.clone(), "GET", "/profiles/not-a-uuid", None).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.media_type, "application/problem+json");

    // And an update of a profile that does not exist creates nothing.
    let answer = send(
        state.clone(),
        "PUT",
        &format!("/profiles/{missing}"),
        Some(a_sound_payload("Fantôme")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    assert_eq!(version_count(&state.pool, missing).await, 0);
}

#[tokio::test]
async fn a_value_outside_the_bounds_is_a_400_and_never_reaches_the_constraint() {
    // The bounds are restated by the API precisely so a client is told which
    // field is wrong instead of receiving a 500 carrying a constraint name. This
    // is what proves the restatement is not merely decorative.
    let name = "a_value_outside_the_bounds_is_a_400_and_never_reaches_the_constraint";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Hors bornes");
    payload["settings"]["weight_kg"] = Value::from(1000.0);
    payload["settings"]["height_cm"] = Value::from(0.0);

    let answer = send(state, "POST", "/profiles", Some(payload)).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.media_type, "application/problem+json");
    assert_eq!(answer.body["type"], "/problems/validation_failed");
    let offending: Vec<&str> = answer.body["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("no errors[] in {}", answer.body))
        .iter()
        .filter_map(|error| error["field"].as_str())
        .collect();
    assert_eq!(offending, vec!["settings.weight_kg", "settings.height_cm"]);
}

#[tokio::test]
async fn the_settings_served_are_the_version_in_force_and_not_the_oldest() {
    // A profile with a history, which is the case `profile_settings_at(id,
    // now())` exists for. Reading the oldest version instead of the one in force
    // would go unnoticed on any profile that owns a single one — that is, on
    // every other test of this file.
    let name = "the_settings_served_are_the_version_in_force_and_not_the_oldest";
    let Some(state) = migrated(name).await else {
        return;
    };

    let (id, _) = create_profile(&state, &a_sound_payload("Deux versions")).await;
    let older = "2020-01-01T00:00:00Z";
    sqlx::query(
        "INSERT INTO public.profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, $3::timestamptz, $4, $5, $6::public.sex, $7::date)",
    )
    .bind(api::new_id())
    .bind(id)
    .bind(older)
    .bind(51.0)
    .bind(160.0)
    .bind("female")
    .bind("1994-05-12")
    .execute(&state.pool)
    .await
    .expect("the older version must be written");

    let answer = send(state, "GET", &format!("/profiles/{id}"), None).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.body["settings"]["weight_kg"], 62.0,
        "the version in force now is the latest one, not the oldest: {}",
        answer.body
    );
}

/// Writes one settings version straight to the database, bypassing the API, and
/// says whether the schema accepted it.
///
/// Used to ask the live catalogue where a bound actually sits, rather than
/// looking for a clause in the migration text — which cannot say which of
/// several accumulated clauses is the one in force.
async fn schema_accepts_version(
    pool: &sqlx::PgPool,
    profile: Uuid,
    weight_kg: f64,
    height_cm: f64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO public.profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, now() - ($3 || ' seconds')::interval, $4, $5, 'female'::public.sex,
                   '1994-05-12'::date)",
    )
    .bind(api::new_id())
    .bind(profile)
    // A distinct `valid_from` per attempt, so the unique constraint on the pair
    // never stands in for the bound under test.
    .bind(fastrand_seconds())
    .bind(weight_kg)
    .bind(height_cm)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// A different whole number of seconds on each call, from the process clock.
fn fastrand_seconds() -> String {
    use std::sync::atomic::{AtomicI64, Ordering};
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed).to_string()
}

/// Writes one profile row straight to the database and says whether the schema
/// accepted it.
async fn schema_accepts_profile(
    pool: &sqlx::PgPool,
    display_name: &str,
    ingestion_seconds: i32,
    absorption_seconds: i32,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO public.profile (
             id, display_name, default_quantity_unit,
             default_ingestion_duration_seconds, default_absorption_duration_seconds
         ) VALUES ($1, $2, 'cl'::public.quantity_unit, $3, $4)",
    )
    .bind(api::new_id())
    .bind(display_name)
    .bind(ingestion_seconds)
    .bind(absorption_seconds)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[tokio::test]
async fn the_schema_enforces_exactly_the_bounds_the_api_restates() {
    // The API restates the schema's bounds so that a bad value comes back as a
    // 400 naming the field instead of a 500 carrying a constraint name. That is
    // only true while the two agree, and this is where they are pinned together.
    //
    // Each bound is probed twice, on the live catalogue and not on the migration
    // text: the value *at* the bound must be refused, and the value immediately
    // inside it accepted. Refused-at pins the schema from being looser than the
    // constant; accepted-just-inside pins it from being stricter — which is the
    // direction that produces a 500, because the API would have let the value
    // through. Together they say the two numbers are the same number.
    //
    // `next_up` / `next_down` are used rather than an arbitrary epsilon: the
    // probe then sits at the closest `f64` there is to the bound, so no gap
    // between the two sides can hide in the interval.
    let name = "the_schema_enforces_exactly_the_bounds_the_api_restates";
    let Some(state) = migrated(name).await else {
        return;
    };
    let pool = &state.pool;

    let (owner, _) = create_profile(&state, &a_sound_payload("Bornes")).await;

    let inside_weight = |w: f64| (w, HEIGHT_CM_MIN_EXCLUSIVE.next_up());
    let inside_height = |h: f64| (WEIGHT_KG_MIN_EXCLUSIVE.next_up(), h);

    for (label, (weight, height), accepted) in [
        (
            "weight at its lower bound",
            inside_weight(WEIGHT_KG_MIN_EXCLUSIVE),
            false,
        ),
        (
            "weight just inside its lower bound",
            inside_weight(WEIGHT_KG_MIN_EXCLUSIVE.next_up()),
            true,
        ),
        (
            "weight at its upper bound",
            inside_weight(WEIGHT_KG_MAX_EXCLUSIVE),
            false,
        ),
        (
            "weight just inside its upper bound",
            inside_weight(WEIGHT_KG_MAX_EXCLUSIVE.next_down()),
            true,
        ),
        (
            "height at its lower bound",
            inside_height(HEIGHT_CM_MIN_EXCLUSIVE),
            false,
        ),
        (
            "height just inside its lower bound",
            inside_height(HEIGHT_CM_MIN_EXCLUSIVE.next_up()),
            true,
        ),
        (
            "height at its upper bound",
            inside_height(HEIGHT_CM_MAX_EXCLUSIVE),
            false,
        ),
        (
            "height just inside its upper bound",
            inside_height(HEIGHT_CM_MAX_EXCLUSIVE.next_down()),
            true,
        ),
    ] {
        let outcome = schema_accepts_version(pool, owner, weight, height).await;
        assert_eq!(
            outcome.is_ok(),
            accepted,
            "{label} ({weight} kg, {height} cm): the schema and the API disagree on where \
             the bound sits — {outcome:?}"
        );
    }

    // The two duration bounds, on the profile itself.
    for (label, ingestion, absorption, accepted) in [
        (
            "ingestion at its lower bound",
            INGESTION_DURATION_MIN_SECONDS,
            1800,
            true,
        ),
        (
            "ingestion below its lower bound",
            INGESTION_DURATION_MIN_SECONDS - 1,
            1800,
            false,
        ),
        (
            "absorption at its exclusive lower bound",
            1200,
            ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS,
            false,
        ),
        (
            "absorption just above it",
            1200,
            ABSORPTION_DURATION_MIN_EXCLUSIVE_SECONDS + 1,
            true,
        ),
    ] {
        let outcome = schema_accepts_profile(pool, "Bornes durées", ingestion, absorption).await;
        // A profile written without a version breaks the deferred trigger of the
        // schema, which fires at COMMIT — but sqlx runs each of these outside a
        // transaction of its own, so the statement commits alone and the trigger
        // rejects it. Only the CHECK violations are of interest here, so the
        // trigger's own message is read as an acceptance of the durations.
        let refused_by_the_check = outcome
            .as_ref()
            .err()
            .is_some_and(|message| message.contains("duration_seconds"));
        assert_eq!(
            !refused_by_the_check, accepted,
            "{label} ({ingestion} s, {absorption} s): the schema and the API disagree — \
             {outcome:?}"
        );
    }

    // And the one clause the API restates without a constant of its own: a blank
    // display name. `validation.rs` trims and refuses it; the schema has to as
    // well, or the API's answer would be a 500.
    let blank = schema_accepts_profile(pool, "   ", 1200, 1800).await;
    assert!(
        blank
            .as_ref()
            .err()
            .is_some_and(|message| message.contains("display_name")),
        "the schema accepts a blank display name: {blank:?}"
    );
}

#[tokio::test]
async fn an_unusually_small_body_is_accepted_and_the_engine_is_left_unguarded() {
    // This test records a **deliberate hole**, so that #16 finds it stated
    // rather than has to rediscover it. Read the module documentation of
    // `profile::validation` with it.
    //
    // Weight and height are bounded away from zero and nothing more, which is a
    // product decision taken by the user on 2026-08-23: accept as many real
    // bodies as possible — someone with dwarfism among them — rather than impose
    // an anthropometric floor. A floor tuned until the Watson equation stopped
    // misbehaving would describe an equation, not a person.
    //
    // The cost is measured and is not zero. The male equation of SPEC.md §6.2
    // reaches a non-positive total body water at 25.715 years for a body at the
    // worst corner this accepts, and `initial_bac_g_per_l` then divides by it:
    // a body of 0.5 kg and 1 cm at 30.6 years gives a total body water of
    // -0.1894 L and, for a single 25 cL beer at 5 %, -41.97 g/L. Those numbers
    // come from `crates/domain` itself.
    //
    // So the request below is accepted, on purpose, and nothing in this crate
    // will stop the curve that follows. The guard that does is `TBW > 0` inside
    // `crates/domain` — issue #16. **When #16 lands, this test is the one to
    // revisit**: what changes is what the engine does with the profile, not
    // whether the profile may exist.
    let name = "an_unusually_small_body_is_accepted_and_the_engine_is_left_unguarded";
    let Some(state) = migrated(name).await else {
        return;
    };

    let mut payload = a_sound_payload("Très petit gabarit");
    payload["settings"]["weight_kg"] = Value::from(0.5);
    payload["settings"]["height_cm"] = Value::from(1.0);
    payload["settings"]["sex"] = Value::from("male");

    let answer = send(state, "POST", "/profiles", Some(payload)).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "no anthropometric floor may turn a real body away: {}",
        answer.body
    );
    assert_eq!(answer.body["settings"]["weight_kg"], 0.5);
    assert_eq!(answer.body["settings"]["height_cm"], 1.0);
}

#[tokio::test]
async fn a_non_positive_weight_or_height_is_still_refused() {
    // The one thing the floors do say. Zero and below are not small bodies, they
    // are not bodies: the Watson equation of SPEC.md §6.2 is linear in both, and
    // a negative weight would flip the sign of the estimate outright.
    let name = "a_non_positive_weight_or_height_is_still_refused";
    let Some(state) = migrated(name).await else {
        return;
    };

    for (weight, height) in [(0.0, 168.0), (-1.0, 168.0), (62.0, 0.0), (62.0, -1.0)] {
        let mut payload = a_sound_payload("Non positif");
        payload["settings"]["weight_kg"] = Value::from(weight);
        payload["settings"]["height_cm"] = Value::from(height);

        let answer = send(state.clone(), "POST", "/profiles", Some(payload)).await;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "({weight} kg, {height} cm) reached the database: {}",
            answer.body
        );
        assert_eq!(answer.media_type, "application/problem+json");
        assert_eq!(answer.body["type"], "/problems/validation_failed");
    }
}

/// Asks one connection of a pool what it carries, rather than watching for a
/// consequence. Two assertions, in this order:
///
/// 1. `SHOW search_path` on that connection equals [`db::SEARCH_PATH`];
/// 2. with a temporary table named `text` and another named `uuid` in place on
///    that same session, `pg_typeof(NULL::text)` and `pg_typeof(NULL::uuid)`
///    still resolve inside `pg_catalog`.
///
/// The second is the property the setting exists for, asked of PostgreSQL's own
/// catalogue: it goes through no migration, no trigger and no statement of this
/// project, so nothing anyone corrects elsewhere can disarm it. That matters
/// here — the end-to-end test below used to be what held this correction, and it
/// bit for a reason its own comment got wrong.
///
/// The caller holds the one connection for the whole check: a temporary table
/// belongs to the session that created it, so a second connection would test
/// nothing and pass regardless. That the two tables really are on this session
/// is asserted rather than assumed. `pool` names the pool in the messages.
async fn assert_pg_temp_named_last(connection: &mut sqlx::PgConnection, pool: &str) {
    let (search_path,): (String,) = sqlx::query_as("SHOW search_path")
        .fetch_one(&mut *connection)
        .await
        .expect("`SHOW search_path` must answer");
    assert_eq!(
        search_path,
        db::SEARCH_PATH,
        "a connection of the pool {pool} builds does not carry the project's \
         search_path"
    );

    sqlx::raw_sql("CREATE TEMP TABLE text (a int); CREATE TEMP TABLE uuid (a int);")
        .execute(&mut *connection)
        .await
        .expect("the shadowing tables must be created");

    // Without this, everything below would pass on a session that never saw the
    // tables — the way a second connection would.
    let (shadowing,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_class
         WHERE relname IN ('text', 'uuid') AND relnamespace = pg_my_temp_schema()",
    )
    .fetch_one(&mut *connection)
    .await
    .expect("the temporary schema must be readable");
    assert_eq!(
        shadowing, 2,
        "the shadowing tables are not on the connection this test then queries"
    );

    for (name, query) in [
        (
            "text",
            "SELECT n.nspname FROM pg_type AS t
             JOIN pg_namespace AS n ON n.oid = t.typnamespace
             WHERE t.oid = pg_typeof(NULL::text)",
        ),
        (
            "uuid",
            "SELECT n.nspname FROM pg_type AS t
             JOIN pg_namespace AS n ON n.oid = t.typnamespace
             WHERE t.oid = pg_typeof(NULL::uuid)",
        ),
    ] {
        let (namespace,): (String,) = sqlx::query_as(query)
            .fetch_one(&mut *connection)
            .await
            .expect("the catalogue must answer");
        assert_eq!(
            namespace, "pg_catalog",
            "`::{name}` resolves into `{namespace}` on a connection of the pool \
             {pool} builds: a temporary table shadows a built-in type"
        );
    }
}

/// A connection of the pool the API runs on names `pg_temp` last.
///
/// **One of the two tests that hold [`db::pool_options`]**: this one takes the
/// pool from [`AppState::new`], the pool the server runs on, and puts one of its
/// connections through [`assert_pg_temp_named_last`].
///
/// Taking the `after_connect` out of `crates/db/src/lib.rs` fails assertion 1
/// with `"$user", public`; putting `PgPoolOptions::new()` back in
/// `crates/api/src/lib.rs` fails the same assertion. Both were run.
#[tokio::test]
async fn the_pool_the_api_builds_names_pg_temp_last_on_every_connection() {
    let test = "the_pool_the_api_builds_names_pg_temp_last_on_every_connection";
    let Some(url) = database_url(test) else {
        return;
    };

    // Through `AppState::new` on purpose: that constructor is the one this test
    // is here to cover.
    let state = state(&url);
    let mut connection = state
        .pool
        .acquire()
        .await
        .expect("the pool must hand out a connection");

    assert_pg_temp_named_last(&mut connection, "`AppState::new`").await;
}

/// A connection of the pool `db::connect()` opens names `pg_temp` last.
///
/// **The other of the two tests that hold [`db::pool_options`]**: this one goes
/// through [`db::connect`] itself — the pool `crates/db/src/main.rs` opens for
/// `migrate`, `reset` and `seed` — and puts one of its connections through
/// [`assert_pg_temp_named_last`]. It does not rebuild that pool from
/// `db::pool_options()`: a test doing so would hold the options, and say
/// nothing of the function meant to use them.
///
/// `db::connect()` reads `DATABASE_URL`, the variable this test is conditioned
/// on, so it opens the same database as the rest of this file.
///
/// Building the pool of `db::connect()` from `PgPoolOptions::new()` instead of
/// `pool_options()` fails assertion 1 with `"$user", public`; so does taking the
/// `after_connect` out of `crates/db/src/lib.rs`. Both were run.
#[tokio::test]
async fn the_pool_db_connect_opens_names_pg_temp_last() {
    let test = "the_pool_db_connect_opens_names_pg_temp_last";
    if database_url(test).is_none() {
        return;
    }

    let pool = db::connect().await.expect("`db::connect()` must open");
    let mut connection = pool
        .acquire()
        .await
        .expect("the pool must hand out a connection");

    assert_pg_temp_named_last(&mut connection, "`db::connect()`").await;
}

/// The four endpoints answer on a connection that owns shadowing temporary
/// tables.
///
/// Kept for its own sake — it exercises the real router, the real statements and
/// the real triggers under the condition this issue is about — but it is **not**
/// what holds the `search_path` correction; the two tests above are. The comment
/// here used to claim otherwise, and was wrong twice over:
///
/// - it announced failing on `operator does not exist: pg_catalog.uuid > uuid`,
///   which can no longer happen: the statements of the module write
///   `$1::pg_catalog.uuid` now, so this correction immunised the very thing the
///   test named;
/// - what it actually bites on, measured on #6 by reading the server log, is
///   `crates/db/migrations/20260819090000_initial_schema.sql`, whose deferred
///   trigger declares `target_profile uuid` unqualified. Without the
///   `after_connect` the failure is `malformed record literal` raised at
///   `COMMIT`, inside `assert_profile_settings_version_present()`, and the
///   module's own `INSERT`s have already succeeded by then.
///
/// That unqualified `DECLARE` is a live class (e) hole and it belongs to
/// **#41**, not to this issue; it is left alone deliberately. But it means this
/// test would go green again the day #41 qualifies that line, which is exactly
/// why it may not be the thing this correction rests on.
///
/// The pool is built by [`db::pool_options`] with **one** connection, which is
/// what puts the requests on the session owning the temporary tables. That is
/// asserted after the requests rather than assumed: the pool must then hold a
/// single connection, and that connection must still own both tables — so the
/// session the requests ran on is the one the tables were created on.
#[tokio::test]
async fn the_endpoints_answer_on_a_connection_that_owns_shadowing_temporary_tables() {
    let test = "the_endpoints_answer_on_a_connection_that_owns_shadowing_temporary_tables";
    let Some(url) = database_url(test) else {
        return;
    };

    let pool = db::pool_options()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("the pool must open");
    db::migrate(&pool).await.expect("the migrations must apply");

    // After the migrations, so that the schema itself is built against the real
    // types; these two shadow only the session this pool holds.
    sqlx::raw_sql("CREATE TEMP TABLE uuid (a int); CREATE TEMP TABLE text (a int);")
        .execute(&pool)
        .await
        .expect("the shadowing tables must be created");

    let state = AppState {
        pool,
        config: state(&url).config,
    };

    // The four endpoints, every one of them on the shadowed connection.
    let (id, _) = create_profile(&state, &a_sound_payload("Ombre")).await;

    let (status, body) = body_of(state.clone(), &format!("/profiles/{id}")).await;
    assert_eq!(status, StatusCode::OK, "the read was refused: {body}");

    let (status, body) = body_of(state.clone(), "/profiles?limit=2").await;
    assert_eq!(status, StatusCode::OK, "the listing was refused: {body}");

    let mut payload = a_sound_payload("Ombre corrigée");
    payload["settings"]["weight_kg"] = Value::from(63.5);
    let pool = state.pool.clone();
    let answer = send(state, "PUT", &format!("/profiles/{id}"), Some(payload)).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the update was refused: {}",
        answer.body
    );

    // What the name of this test claims, checked: one connection served every
    // request, and it is the session that owns the shadowing tables.
    assert_eq!(
        pool.size(),
        1,
        "the requests were spread over more than one connection, so not all of \
         them ran on the session owning the shadowing tables"
    );
    let (shadowing,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_class
         WHERE relname IN ('text', 'uuid') AND relnamespace = pg_my_temp_schema()",
    )
    .fetch_one(&pool)
    .await
    .expect("the temporary schema must be readable");
    assert_eq!(
        shadowing, 2,
        "the connection the requests ran on does not own the shadowing tables"
    );
}

// ---------------------------------------------------------------------------
// Base library (issue #10)
// ---------------------------------------------------------------------------

/// SQL selecting the library rows whose search key starts with that of `$1` —
/// the prefix search the index of the library is built for.
const LIBRARY_PREFIX_SEARCH: &str = "FROM public.library_item
     WHERE public.library_item_search_key(name)
           LIKE public.library_item_search_key($1) || '%'";

/// The names the library search returns for `typed`, in name order.
async fn library_names_starting_with(pool: &sqlx::PgPool, typed: &str) -> Vec<String> {
    sqlx::query_scalar(&format!(
        "SELECT name {LIBRARY_PREFIX_SEARCH} ORDER BY name"
    ))
    .bind(typed)
    .fetch_all(pool)
    .await
    .expect("the library search must run")
}

#[tokio::test]
async fn the_base_library_is_populated_by_the_migrations() {
    // SPEC.md §5.4: the library ships with the application. Nothing but
    // `db::migrate` runs here, so every row found was written by a migration —
    // not by `db::seed`, which no test calls.
    let Some(state) = migrated("the_base_library_is_populated_by_the_migrations").await else {
        return;
    };

    // The drinks issue #10 names, each looked up by the search a user would
    // type, so that a name spelt with accents in the data set is still found.
    // A prefix may also find items of the other category — `gin` finds
    // « Ginger ale » — hence "at least one of", not "only".
    for (typed, category) in [
        ("vodka", "alcohol"),
        ("gin", "alcohol"),
        ("rhum", "alcohol"),
        ("whisky", "alcohol"),
        ("tequila", "alcohol"),
        ("biere", "alcohol"),
        ("vin rouge", "alcohol"),
        ("vin blanc", "alcohol"),
        ("champagne", "alcohol"),
        ("jus d'orange", "soft"),
        ("coca", "soft"),
        ("tonic", "soft"),
        ("jus de citron", "soft"),
        ("sirop", "soft"),
        ("eau gazeuse", "soft"),
    ] {
        let found: Vec<String> =
            sqlx::query_scalar(&format!("SELECT category::text {LIBRARY_PREFIX_SEARCH}"))
                .bind(typed)
                .fetch_all(&state.pool)
                .await
                .expect("the library search must run");
        assert!(
            found.iter().any(|found| found == category),
            "`{typed}` should find a {category} item, found {found:?}"
        );
    }
}

#[tokio::test]
async fn every_soft_is_at_zero_and_every_alcohol_above_it() {
    // SPEC.md §5.4: soft drinks are "à 0° d'alcool". An alcohol at 0 would be
    // the converse accident: a degree forgotten in the data set.
    let Some(state) = migrated("every_soft_is_at_zero_and_every_alcohol_above_it").await else {
        return;
    };

    let misfiled: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT name, category::text, default_abv_percent FROM public.library_item
         WHERE (category = 'soft'::public.library_item_category
                AND default_abv_percent <> 0)
            OR (category = 'alcohol'::public.library_item_category
                AND default_abv_percent <= 0)",
    )
    .fetch_all(&state.pool)
    .await
    .expect("the query must run");
    assert!(misfiled.is_empty(), "{misfiled:?}");
}

#[tokio::test]
async fn searching_biere_finds_biere_whatever_the_accents_and_the_case() {
    // The acceptance criterion of issue #10: « biere » finds « bière ».
    let Some(state) =
        migrated("searching_biere_finds_biere_whatever_the_accents_and_the_case").await
    else {
        return;
    };

    let plain = library_names_starting_with(&state.pool, "biere").await;
    assert!(
        plain.iter().any(|name| name.starts_with("Bière")),
        "`biere` should find a « Bière », found {plain:?}"
    );
    for typed in ["bière", "BIERE", "Bière", "BIÈRE"] {
        assert_eq!(
            library_names_starting_with(&state.pool, typed).await,
            plain,
            "`{typed}` and `biere` should find the same items"
        );
    }
}

#[tokio::test]
async fn the_search_index_serves_the_library_search() {
    // An index the search predicate cannot use would be decoration. The planner
    // rightly prefers a sequential scan on a table this small, so it is told not
    // to, for this transaction only: what is checked is that the index *can*
    // answer the predicate, which takes both the same expression and an operator
    // class that supports `LIKE 'prefix%'` under a non-C collation.
    let Some(state) = migrated("the_search_index_serves_the_library_search").await else {
        return;
    };

    let mut tx = state.pool.begin().await.expect("the transaction must open");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .expect("the planner setting must apply");
    // The value is inlined rather than bound: a generic plan for `$1` could not
    // turn the `LIKE` into an index range, whatever the index.
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT name FROM public.library_item
         WHERE public.library_item_search_key(name)
               LIKE public.library_item_search_key('biere') || '%'",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("the plan must be produced");
    let plan = plan.join("\n");
    assert!(
        plan.contains("Index Cond") && plan.contains("library_item_search_key_unique"),
        "the search does not use the library search index:\n{plan}"
    );
}

/// Tries to add `name` to the library inside the caller's transaction, under a
/// savepoint so that a refusal leaves the transaction usable, and rolls the
/// write back either way. Returns the constraint that refused it, if any.
async fn library_refuses(
    tx: &mut sqlx::PgConnection,
    name: &str,
    category: &str,
) -> Option<String> {
    sqlx::query("SAVEPOINT library_write")
        .execute(&mut *tx)
        .await
        .expect("the savepoint must be set");
    let written = sqlx::query(
        "INSERT INTO public.library_item (id, name, category, default_abv_percent)
         VALUES ($1, $2, $3::public.library_item_category, 0)",
    )
    .bind(api::new_id())
    .bind(name)
    .bind(category)
    .execute(&mut *tx)
    .await;
    let refused_by = match written {
        Ok(_) => None,
        Err(sqlx::Error::Database(error)) => Some(
            error
                .constraint()
                .unwrap_or_else(|| panic!("`{name}` refused by no constraint: {error}"))
                .to_owned(),
        ),
        Err(error) => panic!("unexpected error writing `{name}`: {error}"),
    };
    sqlx::query("ROLLBACK TO SAVEPOINT library_write")
        .execute(&mut *tx)
        .await
        .expect("the savepoint must roll back");
    refused_by
}

#[tokio::test]
async fn a_name_is_unique_across_the_library_whatever_its_accents_case_or_category() {
    // A library name identifies an item for whoever searches it: the search
    // shows names, and a drink component copies the name and the degree, never
    // the category. Two items the search cannot tell apart — « Bière blonde »
    // and « BIERE BLONDE », or a soft and an alcohol of the same name — would
    // offer the user two indistinguishable rows with different degrees.
    let Some(state) =
        migrated("a_name_is_unique_across_the_library_whatever_its_accents_case_or_category").await
    else {
        return;
    };

    // Everything below is rolled back: the library is shared by every test.
    let mut tx = state.pool.begin().await.expect("the transaction must open");
    // The exact name was refused already, by `library_item_lower_name_key` of
    // the initial migration, which still answers first for it.
    assert!(
        library_refuses(&mut tx, "Bière blonde", "alcohol")
            .await
            .is_some(),
        "an exact duplicate must be refused"
    );
    // These two were not: their `lower(name)` differs from « bière blonde ».
    for (name, category) in [("BIERE BLONDE", "alcohol"), ("Biere blonde", "soft")] {
        assert_eq!(
            library_refuses(&mut tx, name, category).await.as_deref(),
            Some("library_item_search_key_unique"),
            "`{name}` ({category}) should be refused as a duplicate of « Bière blonde »"
        );
    }
    assert_eq!(
        library_refuses(&mut tx, "Bière de garde", "alcohol").await,
        None,
        "a new name must still be accepted"
    );
    tx.rollback().await.expect("the transaction must roll back");
}

#[tokio::test]
async fn the_library_identifiers_are_uuid_v7() {
    // The project convention (SPEC.md §10.0): every identifier is a UUID v7,
    // whose ordering the API uses as its pagination cursor. The library rows
    // are written by a migration rather than by the application, so their
    // identifiers are literals of that migration; this is what holds them to
    // the convention.
    let Some(state) = migrated("the_library_identifiers_are_uuid_v7").await else {
        return;
    };

    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM public.library_item ORDER BY id")
        .fetch_all(&state.pool)
        .await
        .expect("the query must run");
    assert!(!ids.is_empty());
    for id in ids {
        assert_eq!(id.get_version_num(), 7, "{id} is not a UUID v7");
        assert_eq!(id.get_variant(), uuid::Variant::RFC4122, "{id}");
    }
}

// ---------------------------------------------------------------------------
// Library search endpoint (issue #11)
// ---------------------------------------------------------------------------

/// Every page of `GET {uri}`, walked by cursor; `uri` carries its own `?`.
async fn library_walk(state: &AppState, uri: &str) -> Vec<Value> {
    let mut items = Vec::new();
    let mut next = uri.to_owned();
    for _ in 0..1000 {
        let answer = send(state.clone(), "GET", &next, None).await;
        assert_eq!(answer.status, StatusCode::OK, "{next}: {}", answer.body);
        items.extend(
            answer.body["items"]
                .as_array()
                .unwrap_or_else(|| panic!("no items[] in {}", answer.body))
                .iter()
                .cloned(),
        );
        match answer.body["next_cursor"].as_str() {
            None => return items,
            Some(cursor) => next = format!("{uri}&cursor={cursor}"),
        }
    }
    panic!("{uri} never reached its last page");
}

/// The names of `GET /library/items?{query}`, every page of it.
async fn library_names(state: &AppState, query: &str) -> Vec<String> {
    library_walk(state, &format!("/library/items?{query}"))
        .await
        .iter()
        .map(|item| {
            item["name"]
                .as_str()
                .expect("an item has a name")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn the_library_search_ignores_case_and_accents() {
    // Acceptance criterion of issue #11.
    let Some(state) = migrated("the_library_search_ignores_case_and_accents").await else {
        return;
    };

    let bieres = vec!["Bière blonde", "Bière blanche", "Bière ambrée"];
    for typed in ["biere", "BIERE", "bi%C3%A8re", "BI%C3%88RE", "Bi%C3%A8re"] {
        assert_eq!(
            library_names(&state, &format!("q={typed}")).await,
            bieres,
            "q={typed}"
        );
    }
    for typed in ["cachaca", "CACHA%C3%87A"] {
        assert_eq!(
            library_names(&state, &format!("q={typed}")).await,
            vec!["Cachaça"],
            "q={typed}"
        );
    }
}

#[tokio::test]
async fn the_library_search_finds_the_text_anywhere_in_the_name() {
    let Some(state) = migrated("the_library_search_finds_the_text_anywhere_in_the_name").await
    else {
        return;
    };

    assert_eq!(
        library_names(&state, "q=orange").await,
        vec!["Jus d'orange"]
    );
    assert_eq!(
        library_names(&state, "q=blonde").await,
        vec!["Bière blonde"]
    );
    // Anywhere means inside a word too: « ROSE » is in « Vin rosé » and in
    // « Prosecco ».
    assert_eq!(
        library_names(&state, "q=ROSE").await,
        vec!["Vin rosé", "Prosecco"]
    );
    // Surrounding blanks are not part of the text looked for.
    assert_eq!(
        library_names(&state, "q=%20%20orange%20").await,
        vec!["Jus d'orange"]
    );
    assert!(library_names(&state, "q=absinthe").await.is_empty());
}

#[tokio::test]
async fn like_wildcards_typed_in_the_library_search_are_plain_characters() {
    // No name of the library holds a `%`, a `_` or a `\`: were the text spliced
    // into a `LIKE` pattern, the first two would match every name.
    let Some(state) =
        migrated("like_wildcards_typed_in_the_library_search_are_plain_characters").await
    else {
        return;
    };

    for typed in ["%25", "_", "%25%25", "%5C", "v_dka", "v%25a"] {
        assert!(
            library_names(&state, &format!("q={typed}"))
                .await
                .is_empty(),
            "q={typed} must match nothing"
        );
    }
}

#[tokio::test]
async fn an_empty_library_search_walks_the_whole_library_in_its_order() {
    // Acceptance criterion of issue #11: an empty request returns a default
    // list. Absent, empty and blank `q` are the same request.
    let Some(state) =
        migrated("an_empty_library_search_walks_the_whole_library_in_its_order").await
    else {
        return;
    };

    let expected: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM public.library_item ORDER BY id")
            .fetch_all(&state.pool)
            .await
            .expect("the query must run");
    assert!(
        expected.len() > usize::from(api::pagination::DEFAULT_LIMIT),
        "the library no longer fills more than one default page; this test \
         would stop exercising the cursor"
    );

    // The first page: the default page size, and a cursor to go on.
    let first = send(state.clone(), "GET", "/library/items", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(
        first.body["items"].as_array().map(Vec::len),
        Some(usize::from(api::pagination::DEFAULT_LIMIT))
    );
    assert!(first.body["next_cursor"].is_string(), "{}", first.body);

    for uri in [
        "/library/items?limit=7",
        "/library/items?q=&limit=7",
        "/library/items?q=%20%20&limit=7",
    ] {
        let walked: Vec<(Uuid, String)> = library_walk(&state, uri)
            .await
            .iter()
            .map(|item| {
                (
                    item["id"]
                        .as_str()
                        .and_then(|id| id.parse().ok())
                        .expect("an item has a UUID"),
                    item["name"]
                        .as_str()
                        .expect("an item has a name")
                        .to_owned(),
                )
            })
            .collect();
        assert_eq!(walked, expected, "{uri}");
    }
}

#[tokio::test]
async fn the_library_is_served_in_identifier_order_whatever_its_physical_order() {
    // The cursor resumes after the identifier of the last item served, so the
    // pages must be in identifier order. Rows freshly written by a migration
    // sit in the heap in that order already, which would hide a missing
    // `ORDER BY`: rewriting a row moves it to the end of the heap. The update
    // changes no value, so the other tests of this file are not disturbed.
    let Some(state) =
        migrated("the_library_is_served_in_identifier_order_whatever_its_physical_order").await
    else {
        return;
    };

    let moved = sqlx::query("UPDATE public.library_item SET name = name WHERE name = 'Vodka'")
        .execute(&state.pool)
        .await
        .expect("the update must run");
    assert_eq!(
        moved.rows_affected(),
        1,
        "« Vodka » is gone from the library"
    );

    let expected: Vec<String> =
        sqlx::query_scalar("SELECT id::text FROM public.library_item ORDER BY id")
            .fetch_all(&state.pool)
            .await
            .expect("the query must run");
    let walked: Vec<String> = library_walk(&state, "/library/items?limit=7")
        .await
        .iter()
        .map(|item| item["id"].as_str().expect("an item has an id").to_owned())
        .collect();
    assert_eq!(walked, expected);
}

#[tokio::test]
async fn the_library_search_filters_by_category() {
    let Some(state) = migrated("the_library_search_filters_by_category").await else {
        return;
    };

    for category in ["alcohol", "soft"] {
        let expected: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM public.library_item WHERE category::text = $1 ORDER BY id",
        )
        .bind(category)
        .fetch_all(&state.pool)
        .await
        .expect("the query must run");
        assert!(!expected.is_empty());
        let walked = library_walk(
            &state,
            &format!("/library/items?category={category}&limit=9"),
        )
        .await;
        assert!(
            walked.iter().all(|item| item["category"] == category),
            "category={category} served another category"
        );
        let names: Vec<String> = walked
            .iter()
            .map(|item| item["name"].as_str().expect("a name").to_owned())
            .collect();
        assert_eq!(names, expected, "category={category}");
    }

    // Combined with a text: `gin` is in « Gin » and in the two gingers.
    assert_eq!(
        library_names(&state, "q=gin").await,
        vec!["Gin", "Ginger ale", "Ginger beer"]
    );
    assert_eq!(
        library_names(&state, "q=gin&category=soft").await,
        vec!["Ginger ale", "Ginger beer"]
    );
    assert_eq!(
        library_names(&state, "q=gin&category=alcohol").await,
        vec!["Gin"]
    );
}

#[tokio::test]
async fn a_library_item_carries_its_identifier_name_degree_and_category() {
    let Some(state) =
        migrated("a_library_item_carries_its_identifier_name_degree_and_category").await
    else {
        return;
    };

    let answer = send(state.clone(), "GET", "/library/items?q=amaretto", None).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        answer.body,
        serde_json::json!({
            "items": [{
                "id": "01a15889-fa80-700e-bfe7-b45e9cc8d9c7",
                "name": "Amaretto",
                "default_abv_percent": 28.0,
                "category": "alcohol",
            }],
            "next_cursor": null,
        })
    );

    let soft = send(state, "GET", "/library/items?q=jus%20d'orange", None).await;
    assert_eq!(soft.body["items"][0]["category"], "soft", "{}", soft.body);
    assert_eq!(soft.body["items"][0]["default_abv_percent"], 0.0);
}

#[tokio::test]
async fn the_library_categories_of_the_api_are_those_of_the_schema() {
    // A label renamed on one side only would turn a category filter into an
    // empty list, and a row read back into a 500.
    let Some(state) = migrated("the_library_categories_of_the_api_are_those_of_the_schema").await
    else {
        return;
    };

    let declared: Vec<String> =
        sqlx::query_scalar("SELECT unnest(enum_range(NULL::public.library_item_category))::text")
            .fetch_all(&state.pool)
            .await
            .expect("the query must run");
    let used: Vec<String> = api::library::Category::ALL
        .iter()
        .map(|category| category.as_sql().to_owned())
        .collect();
    assert_eq!(used, declared);
}

// ---------------------------------------------------------------------------
// Drinks (#14)
// ---------------------------------------------------------------------------

/// A 25 cL beer at 5 %, as a creation payload; the unit is the only member a
/// creation cannot do without beside the name and the components.
fn a_beer(name: &str, ingested_at: &str) -> Value {
    serde_json::json!({
        "name": name,
        "ingested_at": ingested_at,
        "quantity_unit": "cl",
        "components": [{ "name": name, "abv_percent": 5.0, "quantity": 250.0 }],
    })
}

/// Records a drink through the API and answers its identifier and body.
async fn record_drink(state: &AppState, profile: Uuid, payload: &Value) -> (Uuid, Value) {
    let answer = send(
        state.clone(),
        "POST",
        &format!("/profiles/{profile}/drinks"),
        Some(payload.clone()),
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    let id: Uuid = answer.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no identifier in {}", answer.body))
        .parse()
        .expect("the identifier must be a UUID");
    assert_eq!(answer.location.as_deref(), Some(&*format!("/drinks/{id}")));
    (id, answer.body)
}

/// Creates an event the profile takes part in, straight in the database: the
/// event endpoints are #18's.
async fn event_with(pool: &sqlx::PgPool, profile: Uuid) -> Uuid {
    let event = api::new_id();
    sqlx::query("INSERT INTO public.event (id, name, starts_at) VALUES ($1, 'Soirée', now())")
        .bind(event)
        .execute(pool)
        .await
        .expect("the event must be written");
    sqlx::query("INSERT INTO public.event_participant (event_id, profile_id) VALUES ($1, $2)")
        .bind(event)
        .bind(profile)
        .execute(pool)
        .await
        .expect("the participant must be written");
    event
}

/// The drinks of a profile's history, as stored, oldest identifier first.
async fn history_of(pool: &sqlx::PgPool, profile: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM public.drink WHERE profile_id = $1 ORDER BY id")
        .bind(profile)
        .fetch_all(pool)
        .await
        .expect("the history must be readable")
}

async fn component_count(pool: &sqlx::PgPool, drink: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM public.drink_component WHERE drink_id = $1")
        .bind(drink)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}

/// Sum, over the whole stored history of a profile, of the alcohol level each
/// drink brings once absorbed, before elimination — read back through the API
/// and computed with the settings in force at each ingestion time.
///
/// The engine of #16 does not exist yet; this is the part of its input a
/// write of #14 decides, and the part that has to lose a deleted drink.
async fn alcohol_of_history(state: &AppState, profile: Uuid) -> f64 {
    let mut total = 0.0;
    for id in history_of(&state.pool, profile).await {
        let (status, body) = body_of(state.clone(), &format!("/drinks/{id}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let drink: api::drink::Drink = serde_json::from_value(body).expect("a drink");
        let valid = domain::drink::DrinkDraft {
            name: drink.name.clone(),
            unit: drink.quantity_unit.into(),
            volume_total_ml: drink.volume_total_ml,
            ingestion_duration_seconds: drink.ingestion_duration_seconds.into(),
            absorption_duration_seconds: drink.absorption_duration_seconds.into(),
            components: drink
                .components
                .iter()
                .map(|c| domain::drink::ComponentDraft {
                    name: c.name.clone(),
                    abv_percent: c.abv_percent,
                    quantity: c.quantity,
                })
                .collect(),
        }
        .validate()
        .expect("a stored drink is valid");
        let settings = api::profile::store::settings_at(&state.pool, profile, drink.ingested_at)
            .await
            .expect("the lookup must run")
            .expect("a profile always has an applicable version");
        let age_days = (drink.ingested_at.date_naive() - settings.birth_date).num_days();
        let age_years = f64::from(i32::try_from(age_days).expect("fits")) / 365.2425;
        let sex = match settings.sex {
            api::profile::Sex::Male => domain::Sex::Male,
            api::profile::Sex::Female => domain::Sex::Female,
        };
        let water = domain::watson_total_body_water_litres(
            sex,
            settings.weight_kg,
            settings.height_cm,
            age_years,
        );
        total += domain::initial_bac_g_per_l(valid.pure_alcohol_grams(), water);
    }
    total
}

#[tokio::test]
async fn a_drink_entered_from_an_event_is_in_the_history_of_the_profile() {
    // First acceptance criterion of #14.
    let name = "a_drink_entered_from_an_event_is_in_the_history_of_the_profile";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Soirée")).await;
    let event = event_with(&state.pool, profile).await;

    let mut payload = a_beer("Bière blonde", "2026-10-17T21:00:00Z");
    payload["event_id"] = Value::from(event.to_string());
    let (from_event, body) = record_drink(&state, profile, &payload).await;
    assert_eq!(body["event_id"], event.to_string());
    assert_eq!(body["profile_id"], profile.to_string());

    let (from_profile, body) =
        record_drink(&state, profile, &a_beer("Bière", "2026-10-17T22:00:00Z")).await;
    assert!(body["event_id"].is_null(), "{body}");

    assert_eq!(
        history_of(&state.pool, profile).await,
        vec![from_event, from_profile]
    );
}

#[tokio::test]
async fn a_drink_may_not_name_an_event_its_profile_is_not_part_of() {
    let name = "a_drink_may_not_name_an_event_its_profile_is_not_part_of";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Absent")).await;
    let (other, _) = create_profile(&state, &a_sound_payload("Présent")).await;
    let event = event_with(&state.pool, other).await;

    for event_id in [event, api::new_id()] {
        let mut payload = a_beer("Bière", "2026-10-17T21:00:00Z");
        payload["event_id"] = Value::from(event_id.to_string());
        let answer = send(
            state.clone(),
            "POST",
            &format!("/profiles/{profile}/drinks"),
            Some(payload),
        )
        .await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
        assert_eq!(answer.body["errors"][0]["field"], "event_id");
    }
    assert!(history_of(&state.pool, profile).await.is_empty());

    // Nor be moved to one by a modification.
    let (drink, _) = record_drink(&state, profile, &a_beer("Bière", "2026-10-17T21:00:00Z")).await;
    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(serde_json::json!({ "event_id": event.to_string() })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.body["errors"][0]["field"], "event_id");
}

#[tokio::test]
async fn a_drink_takes_its_missing_time_and_durations_from_now_and_the_profile() {
    let name = "a_drink_takes_its_missing_time_and_durations_from_now_and_the_profile";
    let Some(state) = migrated(name).await else {
        return;
    };
    let mut profile_payload = a_sound_payload("Défauts");
    profile_payload["default_ingestion_duration_seconds"] = Value::from(420);
    profile_payload["default_absorption_duration_seconds"] = Value::from(3000);
    let (profile, _) = create_profile(&state, &profile_payload).await;

    let mut payload = a_beer("Bière", "unused");
    payload
        .as_object_mut()
        .expect("an object")
        .remove("ingested_at");
    let before = api::timestamp::now();
    let (_, body) = record_drink(&state, profile, &payload).await;
    let after = api::timestamp::now();

    let ingested_at: api::Timestamp = body["ingested_at"]
        .as_str()
        .expect("an instant")
        .parse()
        .expect("ISO 8601");
    // `timestamptz` keeps microseconds: the stored instant may sit up to one
    // microsecond below the one taken by the handler.
    assert!(
        ingested_at >= before - chrono::Duration::microseconds(1) && ingested_at <= after,
        "{ingested_at} is not the instant of the request ({before} .. {after})"
    );
    assert_eq!(body["ingestion_duration_seconds"], 420);
    assert_eq!(body["absorption_duration_seconds"], 3000);
    assert!(body["feeling_rating"].is_null(), "{body}");
}

#[tokio::test]
async fn a_deleted_drink_no_longer_counts_in_the_alcohol_of_the_profile() {
    // Second acceptance criterion of #14, on the input a recomputation reads.
    let name = "a_deleted_drink_no_longer_counts_in_the_alcohol_of_the_profile";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Deux bières")).await;

    let (kept, _) = record_drink(&state, profile, &a_beer("Bière", "2026-10-17T21:00:00Z")).await;
    let alone = alcohol_of_history(&state, profile).await;
    assert!(alone > 0.0, "{alone}");

    let mut stronger = a_beer("Triple", "2026-10-17T21:30:00Z");
    stronger["components"][0]["abv_percent"] = Value::from(9.0);
    let (deleted, _) = record_drink(&state, profile, &stronger).await;
    let both = alcohol_of_history(&state, profile).await;
    assert!(both > alone + 1e-6, "{both} vs {alone}");

    let answer = send(state.clone(), "DELETE", &format!("/drinks/{deleted}"), None).await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", answer.body);

    assert_eq!(history_of(&state.pool, profile).await, vec![kept]);
    assert_eq!(component_count(&state.pool, deleted).await, 0);
    assert_eq!(
        alcohol_of_history(&state, profile).await.to_bits(),
        alone.to_bits()
    );

    let again = send(state.clone(), "DELETE", &format!("/drinks/{deleted}"), None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.body);
    let (status, _) = body_of(state, &format!("/drinks/{deleted}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_modification_rewrites_what_it_names_and_keeps_the_rest() {
    let name = "a_modification_rewrites_what_it_names_and_keeps_the_rest";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Mixologie")).await;
    let event = event_with(&state.pool, profile).await;
    let mut payload = serde_json::json!({
        "name": "Vodka – Jus d'orange",
        "ingested_at": "2026-10-17T21:00:00Z",
        "quantity_unit": "cl",
        "feeling_rating": 3,
        "event_id": event.to_string(),
        "components": [
            { "name": "Vodka", "abv_percent": 40.0, "quantity": 40.0 },
            { "name": "Jus d'orange", "abv_percent": 0.0, "quantity": 160.0 },
        ],
    });
    let (drink, created) = record_drink(&state, profile, &payload).await;

    // Name and time only: the composition, the rating and the event stay.
    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(serde_json::json!({
            "name": "Screwdriver",
            "ingested_at": "2026-10-17T21:30:00Z",
        })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["name"], "Screwdriver");
    assert_eq!(answer.body["ingested_at"], "2026-10-17T21:30:00Z");
    for kept in ["components", "feeling_rating", "event_id", "created_at"] {
        assert_eq!(answer.body[kept], created[kept], "{kept}");
    }

    // A new composition in percent replaces the whole old one; null clears.
    payload = serde_json::json!({
        "quantity_unit": "percent",
        "volume_total_ml": 200.0,
        "feeling_rating": null,
        "event_id": null,
        "components": [
            { "name": "Gin", "abv_percent": 40.0, "quantity": 25.0 },
            { "name": "Tonic", "abv_percent": 0.0, "quantity": 70.0 },
            { "name": "Citron", "abv_percent": 0.0, "quantity": 5.0 },
        ],
    });
    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["quantity_unit"], "percent");
    assert_eq!(answer.body["volume_total_ml"], 200.0);
    assert!(answer.body["feeling_rating"].is_null(), "{}", answer.body);
    assert!(answer.body["event_id"].is_null(), "{}", answer.body);
    let names: Vec<&str> = answer.body["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|c| c["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["Gin", "Tonic", "Citron"]);
    assert_eq!(component_count(&state.pool, drink).await, 3);
    assert_eq!(answer.body["name"], "Screwdriver");

    let (status, read) = body_of(state, &format!("/drinks/{drink}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, answer.body);
}

#[tokio::test]
async fn an_invalid_write_is_a_400_and_leaves_the_stored_drink_alone() {
    let name = "an_invalid_write_is_a_400_and_leaves_the_stored_drink_alone";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Refus")).await;
    let (drink, created) =
        record_drink(&state, profile, &a_beer("Bière", "2026-10-17T21:00:00Z")).await;

    // Shares above 100 %: refused by the API before the database check.
    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(serde_json::json!({
            "quantity_unit": "percent",
            "volume_total_ml": 200.0,
            "components": [
                { "name": "Gin", "abv_percent": 40.0, "quantity": 60.0 },
                { "name": "Tonic", "abv_percent": 0.0, "quantity": 50.0 },
            ],
        })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.body["errors"][0]["field"], "components");

    let (_, read) = body_of(state.clone(), &format!("/drinks/{drink}")).await;
    assert_eq!(read, created);

    // `null` on a member that cannot be cleared.
    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(serde_json::json!({ "name": null, "components": null })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    let mut fields: Vec<&str> = answer.body["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|e| e["field"].as_str().expect("a field"))
        .collect();
    fields.sort_unstable();
    assert_eq!(fields, ["components", "name"]);
    let (_, read) = body_of(state.clone(), &format!("/drinks/{drink}")).await;
    assert_eq!(read, created);

    // A creation in percent without its total volume.
    let answer = send(
        state.clone(),
        "POST",
        &format!("/profiles/{profile}/drinks"),
        Some(serde_json::json!({
            "name": "Gin tonic",
            "quantity_unit": "percent",
            "components": [{ "name": "Gin", "abv_percent": 40.0, "quantity": 25.0 }],
        })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.body["errors"][0]["field"], "volume_total_ml");
    assert_eq!(history_of(&state.pool, profile).await, vec![drink]);
}

#[tokio::test]
async fn an_ingestion_time_is_held_to_what_the_database_stores() {
    // Ties `earliest_ingested_at` to the live server in both directions: the
    // bound itself is stored and read back unchanged, one microsecond earlier
    // is a field error and not the 500 PostgreSQL would answer.
    let name = "an_ingestion_time_is_held_to_what_the_database_stores";
    let Some(state) = migrated(name).await else {
        return;
    };
    let (profile, _) = create_profile(&state, &a_sound_payload("Préhistoire")).await;
    let earliest = api::drink::validation::earliest_ingested_at();
    let rendered = serde_json::to_value(earliest).expect("must serialise");

    let mut payload = a_beer("Bière", "unused");
    payload["ingested_at"] = rendered.clone();
    let (drink, body) = record_drink(&state, profile, &payload).await;
    assert_eq!(body["ingested_at"], rendered);

    payload["ingested_at"] =
        serde_json::to_value(earliest - chrono::Duration::microseconds(1)).expect("must serialise");
    let answer = send(
        state.clone(),
        "POST",
        &format!("/profiles/{profile}/drinks"),
        Some(payload),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.body["errors"][0]["field"], "ingested_at");

    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{drink}"),
        Some(serde_json::json!({ "ingested_at": "-5000-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    assert_eq!(answer.body["errors"][0]["field"], "ingested_at");
    assert_eq!(history_of(&state.pool, profile).await, vec![drink]);
}

#[tokio::test]
async fn writing_to_an_unknown_profile_or_drink_is_a_404() {
    let Some(state) = migrated("writing_to_an_unknown_profile_or_drink_is_a_404").await else {
        return;
    };
    let unknown = api::new_id();
    let answer = send(
        state.clone(),
        "POST",
        &format!("/profiles/{unknown}/drinks"),
        Some(a_beer("Bière", "2026-10-17T21:00:00Z")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    assert_eq!(answer.media_type, api::error::PROBLEM_JSON);

    let answer = send(
        state.clone(),
        "PATCH",
        &format!("/drinks/{unknown}"),
        Some(serde_json::json!({ "name": "Bière" })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);

    let answer = send(state, "DELETE", &format!("/drinks/{unknown}"), None).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
}
