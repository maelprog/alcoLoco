//! PostgreSQL schema of alcoLoco.
//!
//! This crate owns the migrations (embedded at compile time), the runner that
//! applies them and the development seed. It deliberately holds no query of the
//! business modules: those belong to the modules themselves (SPEC.md §9).

use sqlx::Executor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

/// Environment variable holding the PostgreSQL connection string.
pub const DATABASE_URL_ENV: &str = "DATABASE_URL";

/// Connection string used when [`DATABASE_URL_ENV`] is not set. Matches the
/// defaults of `docker-compose.yml`.
pub const DEFAULT_DATABASE_URL: &str = "postgres://alcoloco:alcoloco@localhost:5432/alcoloco";

/// Every migration of `crates/db/migrations`, embedded in the binary at compile
/// time so that applying them never depends on the current directory.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The `search_path` every connection this project opens runs with.
///
/// `pg_temp` is **named**, and named last. That is the whole point of this
/// constant. Left out of the list — which is what PostgreSQL does by default —
/// the temporary schema is searched **first** for relations *and for types*, so
/// any session may shadow a type a statement casts to:
///
/// ```text
/// SELECT 'x'::text;                  -- x
/// CREATE TEMP TABLE text (a int);
/// SELECT 'x'::text;                  -- ERROR: malformed record literal: "x"
/// ```
///
/// and, when the profile module still wrote `$1::uuid` unqualified, a temporary
/// table named `uuid` turned its comparison into `ERROR: operator does not
/// exist: pg_catalog.uuid > uuid`. Those statements name `pg_catalog` now, so
/// that particular spelling is gone from the module; both errors were measured
/// on `postgres:16-alpine`, the image of `docker-compose.yml`.
///
/// Naming `pg_temp` moves it to the position written here, which settles the
/// resolution for every statement **the two pools built from
/// [`pool_options`]** run — not only the statements someone remembered to
/// qualify, and not only the shapes a text guard can recognise. The
/// qualification guards of `crates/api/src/profile/store.rs` keep a convention
/// worth keeping; they are no longer what stands between a temporary table and
/// a mis-resolved name.
///
/// **What this does not reach**, so that the next reader does not take it for
/// more than it is:
///
/// - a session that runs `DISCARD ALL` gets the default back — measured:
///   `SHOW search_path` returns `"$user", public` straight after it. Nothing in
///   this repository issues one, but a connection pooler in transaction mode
///   does;
/// - any client that is not one of these two pools — `psql`, an operations
///   script, a future crate opening its own pool. For those,
///   `crates/db/migrations/20260819090000_initial_schema.sql` still holds a live
///   class (e) hole of its own: the deferred trigger
///   `assert_profile_settings_version_present()` declares `target_profile uuid`
///   unqualified. It is left alone on purpose — migrations are immutable here,
///   and that line belongs to **#41**.
///
/// Closing the class *by construction* would take `ALTER DATABASE … SET
/// search_path`, which no session can undo and no client can miss. That is
/// #41's to decide; a session setting is what this issue can honestly do.
///
/// One thing this order **moves** rather than removes. PostgreSQL puts
/// `pg_catalog` implicitly first when it is not named; naming it second puts it
/// after `public`, so a type of `public` now shadows a built-in of the same
/// name. Measured:
///
/// ```text
/// SET search_path = public, pg_catalog, pg_temp;
/// CREATE DOMAIN public.text AS pg_catalog.int4;
/// SELECT pg_typeof(NULL::text);   -- public   (pg_catalog under the default)
/// ```
///
/// That is a strictly better place for the risk to sit: shadowing now takes DDL
/// on `public`, where before it took a temporary table any session may create
/// with no privilege at all. And `public` has to come first regardless, or the
/// migrations would create their objects somewhere else.
pub const SEARCH_PATH: &str = "public, pg_catalog, pg_temp";

/// Pool options carrying what every connection of this project needs.
///
/// The project's two pools are built from here — this crate's [`connect`] and
/// the API's state — and a test covers each. Nothing in the language prevents a
/// third being built elsewhere with `PgPoolOptions::new()`, and no guard would
/// notice: what holds this is the two tests and this sentence, not the
/// structure. `AppState::new` cannot simply call [`connect`] instead, because
/// its pool is deliberately lazy — the server must come up and answer
/// `GET /health` with the database down.
#[must_use]
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new().after_connect(|connection, _metadata| {
        Box::pin(async move {
            connection
                .execute(format!("SET search_path = {SEARCH_PATH}").as_str())
                .await?;
            Ok(())
        })
    })
}

/// Connection string to use, from the environment or from the local default.
#[must_use]
pub fn database_url() -> String {
    std::env::var(DATABASE_URL_ENV).unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned())
}

/// Opens a small connection pool on [`database_url`].
///
/// # Errors
///
/// Fails when the server is unreachable or refuses the credentials.
pub async fn connect() -> Result<PgPool, sqlx::Error> {
    pool_options()
        .max_connections(4)
        .connect(&database_url())
        .await
}

/// Applies every migration not yet recorded in `_sqlx_migrations`.
///
/// Running it twice in a row is a no-op: the second run finds every version
/// already recorded and executes no SQL.
///
/// # Errors
///
/// Fails when a migration cannot be applied, or when an already applied
/// migration has been edited since (checksum mismatch).
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// Drops the whole `public` schema and re-applies every migration, leaving an
/// empty but up-to-date database.
///
/// # Errors
///
/// Fails when the schema cannot be dropped, or when a migration fails.
/// A `search_path` naming a schema that does not exist is not an error:
/// PostgreSQL skips the missing entry, so the window between the two statements
/// below is harmless. Probed on #6 rather than assumed — see
/// `crates/api/tests/database.rs`.
pub async fn reset(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .execute(pool)
        .await?;
    migrate(pool).await?;
    Ok(())
}

/// Identifier generated for every row this crate writes.
///
/// Every table takes its primary key from the application: PostgreSQL 16 has no
/// native `uuidv7()`, and no column carries a default. Funnelling the generation
/// through this single function is what makes the convention testable — v7 is
/// time ordered, and the API uses that ordering as its pagination cursor, so a
/// v4 slipped in here would silently break paging rather than fail loudly.
fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Number of profiles already present, used to keep [`seed`] from running twice.
async fn profile_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM profile")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// Inserts a small, coherent development data set.
///
/// Every identifier is a UUID v7 generated here, which is also how the API
/// generates them. Timestamps are relative to `now()` so that the seeded evening
/// is always "a few hours ago" whenever the seed is run.
///
/// # Errors
///
/// Fails when the database is not empty, or on any write error.
pub async fn seed(pool: &PgPool) -> Result<(), sqlx::Error> {
    if profile_count(pool).await? > 0 {
        return Err(sqlx::Error::Protocol(
            "database already holds profiles: run `reset` first".to_owned(),
        ));
    }

    let mut tx = pool.begin().await?;

    // Base library. The real V1 data set is the subject of issue #10; these four
    // rows only exist to make the seeded drinks realistic.
    for (name, category, abv) in [
        ("Vodka", "alcohol", 40.0_f64),
        ("Gin", "alcohol", 37.5),
        ("Jus d'orange", "soft", 0.0),
        ("Coca", "soft", 0.0),
    ] {
        sqlx::query(
            "INSERT INTO library_item (id, name, category, default_abv_percent)
             VALUES ($1, $2, $3::library_item_category, $4)",
        )
        .bind(new_id())
        .bind(name)
        .bind(category)
        .bind(abv)
        .execute(&mut *tx)
        .await?;
    }

    // Alice: a single settings version, so her whole history uses it.
    let alice = new_id();
    insert_profile(&mut tx, alice, "Alice", "cl").await?;
    insert_settings_version(
        &mut tx,
        alice,
        "now() - interval '2 years'",
        62.0,
        168.0,
        "female",
        "1994-05-12",
    )
    .await?;

    // Bob: two versions, to exercise the "parameters in force at ingestion time"
    // rule of SPEC.md §5.6. The oldest one has an open lower bound.
    let bob = new_id();
    insert_profile(&mut tx, bob, "Bob", "percent").await?;
    insert_settings_version(
        &mut tx,
        bob,
        "now() - interval '1 year'",
        75.0,
        180.0,
        "male",
        "1996-03-02",
    )
    .await?;
    insert_settings_version(
        &mut tx,
        bob,
        "now() - interval '2 months'",
        80.0,
        180.0,
        "male",
        "1996-03-02",
    )
    .await?;

    // An open-ended event with both profiles.
    let event = new_id();
    sqlx::query(
        "INSERT INTO event (id, name, starts_at, ends_at)
         VALUES ($1, $2, now() - interval '4 hours', NULL)",
    )
    .bind(event)
    .bind("Soirée de démonstration")
    .execute(&mut *tx)
    .await?;

    for profile in [alice, bob] {
        sqlx::query("INSERT INTO event_participant (event_id, profile_id) VALUES ($1, $2)")
            .bind(event)
            .bind(profile)
            .execute(&mut *tx)
            .await?;
    }

    // A drink measured in cL: the total volume is derived from the components,
    // hence left NULL.
    let beer = insert_drink(
        &mut tx,
        &DrinkSeed {
            profile_id: alice,
            event_id: Some(event),
            name: "Bière blonde",
            ingested_at_sql: "now() - interval '3 hours'",
            unit: "cl",
            volume_total_ml: None,
            feeling_rating: Some(2),
        },
    )
    .await?;
    insert_component(&mut tx, beer, 0, "Bière blonde", 5.0, 250.0).await?;

    // A drink measured in % of its total volume: the total volume is mandatory.
    let cocktail = insert_drink(
        &mut tx,
        &DrinkSeed {
            profile_id: bob,
            event_id: Some(event),
            name: "Gin – Jus d'orange",
            ingested_at_sql: "now() - interval '2 hours'",
            unit: "percent",
            volume_total_ml: Some(200.0),
            feeling_rating: Some(3),
        },
    )
    .await?;
    insert_component(&mut tx, cocktail, 0, "Gin", 37.5, 25.0).await?;
    insert_component(&mut tx, cocktail, 1, "Jus d'orange", 0.0, 75.0).await?;

    // A drink outside of any event: the history of a profile is not limited to
    // its evenings (SPEC.md §4).
    let shot = insert_drink(
        &mut tx,
        &DrinkSeed {
            profile_id: alice,
            event_id: None,
            name: "Vodka",
            ingested_at_sql: "now() - interval '30 hours'",
            unit: "cl",
            volume_total_ml: None,
            feeling_rating: None,
        },
    )
    .await?;
    insert_component(&mut tx, shot, 0, "Vodka", 40.0, 40.0).await?;

    tx.commit().await
}

type Tx<'c> = sqlx::Transaction<'c, sqlx::Postgres>;

async fn insert_profile(
    tx: &mut Tx<'_>,
    id: Uuid,
    display_name: &str,
    default_unit: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO profile (
             id, display_name, default_quantity_unit,
             default_ingestion_duration_seconds, default_absorption_duration_seconds
         ) VALUES ($1, $2, $3::quantity_unit, 1200, 1800)",
    )
    .bind(id)
    .bind(display_name)
    .bind(default_unit)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// `valid_from_sql` is a SQL expression rather than a bound value so that the
/// seed stays free of any date-time dependency.
async fn insert_settings_version(
    tx: &mut Tx<'_>,
    profile_id: Uuid,
    valid_from_sql: &str,
    weight_kg: f64,
    height_cm: f64,
    sex: &str,
    birth_date: &str,
) -> Result<(), sqlx::Error> {
    let statement = format!(
        "INSERT INTO profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, {valid_from_sql}, $3, $4, $5::sex, $6::date)"
    );
    sqlx::query(&statement)
        .bind(new_id())
        .bind(profile_id)
        .bind(weight_kg)
        .bind(height_cm)
        .bind(sex)
        .bind(birth_date)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// One seeded drink. Grouped in a struct rather than passed as eight positional
/// arguments, which no reader could keep straight.
struct DrinkSeed<'a> {
    profile_id: Uuid,
    event_id: Option<Uuid>,
    name: &'a str,
    /// SQL expression for `ingested_at`, e.g. `now() - interval '3 hours'`.
    ingested_at_sql: &'a str,
    unit: &'a str,
    volume_total_ml: Option<f64>,
    feeling_rating: Option<i16>,
}

async fn insert_drink(tx: &mut Tx<'_>, drink: &DrinkSeed<'_>) -> Result<Uuid, sqlx::Error> {
    let id = new_id();
    let statement = format!(
        "INSERT INTO drink (
             id, profile_id, event_id, name, ingested_at,
             ingestion_duration_seconds, absorption_duration_seconds,
             quantity_unit, volume_total_ml, feeling_rating
         ) VALUES ($1, $2, $3, $4, {}, 1200, 1800, $5::quantity_unit, $6, $7)",
        drink.ingested_at_sql
    );
    sqlx::query(&statement)
        .bind(id)
        .bind(drink.profile_id)
        .bind(drink.event_id)
        .bind(drink.name)
        .bind(drink.unit)
        .bind(drink.volume_total_ml)
        .bind(drink.feeling_rating)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

async fn insert_component(
    tx: &mut Tx<'_>,
    drink_id: Uuid,
    sort_order: i32,
    name: &str,
    abv_percent: f64,
    quantity: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO drink_component (id, drink_id, sort_order, name, abv_percent, quantity)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new_id())
    .bind(drink_id)
    .bind(sort_order)
    .bind(name)
    .bind(abv_percent)
    .bind(quantity)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_ships_at_least_one_migration() {
        assert!(!MIGRATOR.migrations.is_empty());
    }

    #[test]
    fn migration_versions_are_unique_and_increasing() {
        let versions: Vec<i64> = MIGRATOR.migrations.iter().map(|m| m.version).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(versions, sorted, "migrations must be uniquely versioned");
    }

    #[test]
    fn migrations_are_forward_only() {
        // A down migration would let a merged migration be rewritten, which the
        // project forbids: migrations are append-only once on `main`.
        assert!(
            MIGRATOR
                .migrations
                .iter()
                .all(|m| m.migration_type.is_up_migration())
        );
    }

    #[test]
    fn the_identifiers_this_crate_generates_are_uuid_v7() {
        // Guards `new_id`, through which every identifier written by this crate
        // is generated — not the `uuid` crate, which is not ours to test.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let id = new_id();
            assert_eq!(id.get_version_num(), 7, "{id} is not a UUID v7");
            assert_eq!(id.get_variant(), uuid::Variant::RFC4122, "{id}");
            assert!(seen.insert(id), "{id} was generated twice");
        }
    }

    #[test]
    fn the_identifiers_this_crate_generates_sort_in_generation_order() {
        // The property the API relies on: v7 is time ordered, which is what makes
        // an identifier usable as a pagination cursor.
        //
        // Checked on a long run of consecutive draws rather than on two draws
        // separated by a sleep. A sleep only exercises the easy case, two
        // identifiers born in distinct milliseconds; the draws the API actually
        // makes are bursts inside a single millisecond, where the ordering rests
        // on the counter v7 keeps there. This form is also deterministic: it
        // waits for nothing and asserts on no clock.
        let ids: Vec<Uuid> = (0..10_000).map(|_| new_id()).collect();
        for pair in ids.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{} should sort before {}",
                pair[0],
                pair[1]
            );
        }
    }

    /// `docker-compose.yml` of the repository root, read at compile time so that
    /// the test below compares the real file rather than a copy of it.
    const DOCKER_COMPOSE: &str = include_str!("../../../docker-compose.yml");

    /// Default value of `${KEY:-default}` as written in `docker-compose.yml`.
    ///
    /// Every occurrence is collected, not just the first: the compose file
    /// repeats `POSTGRES_USER` and `POSTGRES_DB` in the healthcheck of the `db`
    /// service, and reading only the first occurrence would let the file
    /// contradict itself — a healthcheck probing a user the server never
    /// creates — without this test noticing. Disagreeing defaults are a failure
    /// here, so the value returned is unambiguous.
    fn compose_default(key: &str) -> &'static str {
        let marker = format!("${{{key}:-");
        let defaults: Vec<&'static str> = DOCKER_COMPOSE
            .split(marker.as_str())
            .skip(1)
            .map(|after| {
                after
                    .split_once('}')
                    .unwrap_or_else(|| panic!("unterminated `{marker}` in docker-compose.yml"))
                    .0
            })
            .collect();

        let (value, others) = defaults
            .split_first()
            .unwrap_or_else(|| panic!("docker-compose.yml defines no `{marker}...}}`"));
        assert!(
            others.iter().all(|other| other == value),
            "docker-compose.yml gives `{marker}...}}` several different defaults: {defaults:?}"
        );
        value
    }

    #[test]
    fn the_default_database_url_matches_docker_compose() {
        // The default connection string exists to make `cargo run -p db` work
        // against the compose stack without any environment variable. It is only
        // useful while it stays equal to what the compose file actually starts,
        // so the whole string is rebuilt from that file instead of being spot
        // checked: user, password, published port and database name all move
        // together or the test fails.
        let user = compose_default("POSTGRES_USER");
        let password = compose_default("POSTGRES_PASSWORD");
        let database = compose_default("POSTGRES_DB");
        let port = compose_default("POSTGRES_PORT");

        assert_eq!(
            DEFAULT_DATABASE_URL,
            format!("postgres://{user}:{password}@localhost:{port}/{database}"),
            "DEFAULT_DATABASE_URL no longer matches docker-compose.yml"
        );
    }
}
