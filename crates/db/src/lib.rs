//! PostgreSQL schema of alcoLoco.
//!
//! This crate owns the migrations (embedded at compile time), the runner that
//! applies them and the development seed. It deliberately holds no query of the
//! business modules: those belong to the modules themselves (SPEC.md §9).

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
    PgPoolOptions::new()
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
pub async fn reset(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .execute(pool)
        .await?;
    migrate(pool).await?;
    Ok(())
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
        .bind(Uuid::now_v7())
        .bind(name)
        .bind(category)
        .bind(abv)
        .execute(&mut *tx)
        .await?;
    }

    // Alice: a single settings version, so her whole history uses it.
    let alice = Uuid::now_v7();
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
    let bob = Uuid::now_v7();
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
    let event = Uuid::now_v7();
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
        .bind(Uuid::now_v7())
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
    let id = Uuid::now_v7();
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
    .bind(Uuid::now_v7())
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
    fn generated_identifiers_are_uuid_v7() {
        // The whole schema relies on the application supplying v7 identifiers.
        assert_eq!(Uuid::now_v7().get_version_num(), 7);
    }

    #[test]
    fn the_default_database_url_matches_docker_compose() {
        assert!(DEFAULT_DATABASE_URL.starts_with("postgres://alcoloco:alcoloco@"));
    }
}
