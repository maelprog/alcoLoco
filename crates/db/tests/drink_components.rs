//! The structural rules of a drink's components, as the database enforces them
//! (issue #13, SPEC.md §4, §5.3, §10.0-A): a drink has one or more components,
//! the components of a drink in percent sum to at most 100 %, and a quantity or
//! a total volume is a finite number.
//!
//! Like `settings_version_concurrency.rs`, every test needs a real PostgreSQL,
//! skips itself when `DATABASE_URL` is not set, and `ALCOLOCO_REQUIRE_DB=1`
//! turns that skip into a failure:
//!
//! ```text
//! ALCOLOCO_REQUIRE_DB=1 DATABASE_URL=postgres://… cargo test -p db --test drink_components
//! ```
//!
//! The concurrent scenarios run two transactions whose snapshots are both taken
//! before either commit, in a fixed commit order. They also need those two
//! transactions to be the only ones in flight: under `SERIALIZABLE`,
//! PostgreSQL detects conflicts through predicate locks that may cover more
//! than the rows read, a whole `drink_component` relation when the check reads
//! it by sequential scan, as it does on a small table. Two `SERIALIZABLE`
//! transactions writing components of different drinks can then fail one of
//! them with 40001 (`could not serialize access due to read/write dependencies
//! among transactions`): a false positive PostgreSQL documents, and which a
//! client retries. The tests of this file, run in parallel threads, would
//! otherwise inflict it on one another: each pair runs under an advisory lock,
//! [`PAIRS_LOCK`], that keeps any other pair of this file, from this process
//! or another one on the same database, out of its way. Within a pair, the one
//! test expecting two `SERIALIZABLE` transactions on different drinks to both
//! commit replays, as a client would, a transaction refused that way.

use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};
use sqlx::{Connection, Executor};
use uuid::Uuid;

const REQUIRE_DATABASE_ENV: &str = "ALCOLOCO_REQUIRE_DB";

/// SQLSTATE of the checks' own refusals (`integrity_constraint_violation`).
const CHECK_REFUSED: &str = "23000";

/// SQLSTATE `check_violation`, raised by a table CHECK constraint.
const CHECK_VIOLATION: &str = "23514";

/// SQLSTATE `serialization_failure`.
const SERIALIZATION_FAILURE: &str = "40001";

const ISOLATION_LEVELS: [&str; 3] = ["READ COMMITTED", "REPEATABLE READ", "SERIALIZABLE"];

fn skipping_is_forbidden() -> bool {
    std::env::var(REQUIRE_DATABASE_ENV)
        .is_ok_and(|value| !value.trim().is_empty() && value.trim() != "0")
}

fn database_url(test: &str) -> Option<String> {
    match std::env::var(db::DATABASE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            assert!(
                !skipping_is_forbidden(),
                "`{test}` may not skip: {REQUIRE_DATABASE_ENV} is set but {} is not",
                db::DATABASE_URL_ENV
            );
            eprintln!(
                "skipping `{test}`: {} is not set (only visible under `-- --nocapture`)",
                db::DATABASE_URL_ENV
            );
            None
        }
    }
}

async fn migrated_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .expect("the database must be reachable");
    db::migrate(&pool).await.expect("the migrations must apply");
    pool
}

async fn connection(url: &str) -> PgConnection {
    PgConnection::connect(url)
        .await
        .expect("the database must be reachable")
}

async fn run(conn: &mut PgConnection, sql: &str) -> Result<(), sqlx::Error> {
    conn.execute(sql).await.map(|_| ())
}

/// Runs `sql` on its own connection, in one transaction.
async fn run_alone(url: &str, sql: &str) -> Result<(), sqlx::Error> {
    let mut conn = connection(url).await;
    run(&mut conn, "BEGIN").await.expect("BEGIN");
    run(&mut conn, sql).await?;
    run(&mut conn, "COMMIT").await
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.into_owned())
}

fn assert_refused(outcome: &Result<(), sqlx::Error>, code: &str, message: &str) {
    let error = outcome
        .as_ref()
        .expect_err("the transaction must be refused");
    assert_eq!(
        sqlstate(error).as_deref(),
        Some(code),
        "wrong mechanism: {error}"
    );
    assert!(
        error.to_string().contains(message),
        "wrong message, expected `{message}`: {error}"
    );
}

fn no_component(drink: Uuid) -> String {
    format!("drink {drink} has no component")
}

fn above_100(drink: Uuid) -> String {
    format!("components of drink {drink} exceed 100 %")
}

/// A profile with one settings version, committed.
async fn profile(pool: &PgPool) -> Uuid {
    let profile = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("a transaction must open");
    sqlx::query(
        "INSERT INTO profile (
             id, display_name, default_quantity_unit,
             default_ingestion_duration_seconds, default_absorption_duration_seconds
         ) VALUES ($1, 'drinks', 'cl', 60, 60)",
    )
    .bind(profile)
    .execute(&mut *tx)
    .await
    .expect("the profile must insert");
    sqlx::query(
        "INSERT INTO profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ($1, $2, timestamptz '2020-01-01', 70, 170, 'male', date '1990-01-01')",
    )
    .bind(Uuid::now_v7())
    .bind(profile)
    .execute(&mut *tx)
    .await
    .expect("the version must insert");
    tx.commit().await.expect("the fixture must commit");
    profile
}

fn insert_drink_sql(
    drink: Uuid,
    profile: Uuid,
    unit: &str,
    volume_total_ml: Option<f64>,
) -> String {
    let volume = volume_total_ml.map_or_else(|| "NULL".to_owned(), |v| format!("{v:?}"));
    format!(
        "INSERT INTO drink (
             id, profile_id, name, ingested_at,
             ingestion_duration_seconds, absorption_duration_seconds,
             quantity_unit, volume_total_ml
         ) VALUES ('{drink}', '{profile}', 'drink', timestamptz '2026-01-01', 600, 1800,
                   '{unit}', {volume});"
    )
}

fn insert_component_sql(component: Uuid, drink: Uuid, sort_order: usize, quantity: &str) -> String {
    format!(
        "INSERT INTO drink_component (id, drink_id, sort_order, name, abv_percent, quantity)
         VALUES ('{component}', '{drink}', {sort_order}, 'component', 40, {quantity}::float8);"
    )
}

/// Commits a drink with one component per quantity and returns the drink id
/// followed by the component ids, in order.
async fn drink_with(
    url: &str,
    profile: Uuid,
    unit: &str,
    volume_total_ml: Option<f64>,
    quantities: &[f64],
) -> (Uuid, Vec<Uuid>) {
    let drink = Uuid::now_v7();
    let mut sql = insert_drink_sql(drink, profile, unit, volume_total_ml);
    let mut ids = Vec::new();
    for (order, quantity) in quantities.iter().enumerate() {
        let id = Uuid::now_v7();
        sql.push_str(&insert_component_sql(
            id,
            drink,
            order,
            &format!("{quantity:?}"),
        ));
        ids.push(id);
    }
    run_alone(url, &sql)
        .await
        .unwrap_or_else(|error| panic!("the fixture must commit: {error}"));
    (drink, ids)
}

async fn component_count(pool: &PgPool, drink: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM drink_component WHERE drink_id = $1")
        .bind(drink)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}

async fn percent_sum(pool: &PgPool, drink: Uuid) -> f64 {
    sqlx::query_scalar("SELECT coalesce(sum(quantity), 0) FROM drink_component WHERE drink_id = $1")
        .bind(drink)
        .fetch_one(pool)
        .await
        .expect("the sum must run")
}

// ----- One or more components, a transaction running alone -----------------

#[tokio::test]
async fn a_drink_committed_without_component_is_refused() {
    let Some(url) = database_url("a_drink_committed_without_component_is_refused") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for unit in [("cl", None), ("percent", Some(330.0))] {
        let drink = Uuid::now_v7();
        let outcome = run_alone(&url, &insert_drink_sql(drink, profile, unit.0, unit.1)).await;
        assert_refused(&outcome, CHECK_REFUSED, &no_component(drink));
    }
}

#[tokio::test]
async fn a_drink_with_components_commits_in_both_units() {
    let Some(url) = database_url("a_drink_with_components_commits_in_both_units") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    // cL quantities are volumes (centilitres since migration 20261022090000):
    // 300 in total is no percentage.
    let (cl, _) = drink_with(&url, profile, "cl", None, &[200.0, 100.0]).await;
    assert_eq!(component_count(&pool, cl).await, 2);

    let (percent, _) = drink_with(&url, profile, "percent", Some(200.0), &[25.0, 75.0]).await;
    assert_eq!(component_count(&pool, percent).await, 2);
}

#[tokio::test]
async fn removing_the_last_component_is_refused_and_removing_another_is_not() {
    let Some(url) =
        database_url("removing_the_last_component_is_refused_and_removing_another_is_not")
    else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, components) = drink_with(&url, profile, "cl", None, &[40.0, 160.0]).await;

    run_alone(
        &url,
        &format!("DELETE FROM drink_component WHERE id = '{}'", components[0]),
    )
    .await
    .expect("a drink keeping a component commits");
    assert_eq!(component_count(&pool, drink).await, 1);

    let outcome = run_alone(
        &url,
        &format!("DELETE FROM drink_component WHERE id = '{}'", components[1]),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &no_component(drink));
    assert_eq!(component_count(&pool, drink).await, 1);
}

#[tokio::test]
async fn replacing_every_component_in_one_transaction_commits() {
    // The check is deferred: what counts is the state at COMMIT, not the
    // empty one the transaction goes through.
    let Some(url) = database_url("replacing_every_component_in_one_transaction_commits") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, _) = drink_with(&url, profile, "cl", None, &[40.0]).await;

    let sql = format!(
        "DELETE FROM drink_component WHERE drink_id = '{drink}'; {}",
        insert_component_sql(Uuid::now_v7(), drink, 0, "50")
    );
    run_alone(&url, &sql)
        .await
        .expect("the replacement commits");
    assert_eq!(component_count(&pool, drink).await, 1);
}

#[tokio::test]
async fn moving_the_only_component_away_is_refused() {
    let Some(url) = database_url("moving_the_only_component_away_is_refused") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (from, components) = drink_with(&url, profile, "cl", None, &[40.0]).await;
    let (to, _) = drink_with(&url, profile, "cl", None, &[40.0]).await;

    let outcome = run_alone(
        &url,
        &format!(
            "UPDATE drink_component SET drink_id = '{to}', sort_order = 1 WHERE id = '{}'",
            components[0]
        ),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &no_component(from));
}

#[tokio::test]
async fn renumbering_a_drink_emptied_in_the_same_transaction_is_refused() {
    // The foreign key refuses to renumber a drink a component still points at,
    // not one whose components were deleted first: the check of the deleted
    // components then looks for an id that names no drink any more.
    let Some(url) = database_url("renumbering_a_drink_emptied_in_the_same_transaction_is_refused")
    else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, _) = drink_with(&url, profile, "cl", None, &[40.0]).await;
    let renumbered = Uuid::now_v7();

    let outcome = run_alone(
        &url,
        &format!(
            "DELETE FROM drink_component WHERE drink_id = '{drink}';
             UPDATE drink SET id = '{renumbered}' WHERE id = '{drink}'"
        ),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &no_component(renumbered));
    assert_eq!(component_count(&pool, drink).await, 1);
}

#[tokio::test]
async fn deleting_a_drink_cascades_to_its_components() {
    let Some(url) = database_url("deleting_a_drink_cascades_to_its_components") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, _) = drink_with(&url, profile, "percent", Some(200.0), &[25.0, 75.0]).await;

    run_alone(&url, &format!("DELETE FROM drink WHERE id = '{drink}'"))
        .await
        .expect("deleting a drink commits");
    assert_eq!(component_count(&pool, drink).await, 0);

    // And so does deleting the profile, through the drink.
    let (drink, _) = drink_with(&url, profile, "cl", None, &[40.0]).await;
    run_alone(&url, &format!("DELETE FROM profile WHERE id = '{profile}'"))
        .await
        .expect("deleting a profile commits");
    assert_eq!(component_count(&pool, drink).await, 0);
}

// ----- At most 100 % in percent, a transaction running alone ---------------

#[tokio::test]
async fn components_in_percent_above_100_are_refused() {
    let Some(url) = database_url("components_in_percent_above_100_are_refused") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    // The case reproduced on the initial schema: 200 % + 100 % of 500 mL.
    let drink = Uuid::now_v7();
    let sql = format!(
        "{}{}{}",
        insert_drink_sql(drink, profile, "percent", Some(500.0)),
        insert_component_sql(Uuid::now_v7(), drink, 0, "200"),
        insert_component_sql(Uuid::now_v7(), drink, 1, "100"),
    );
    assert_refused(
        &run_alone(&url, &sql).await,
        CHECK_REFUSED,
        &above_100(drink),
    );

    // Just above: the tolerance is for rounding, not for a real excess.
    let drink = Uuid::now_v7();
    let sql = format!(
        "{}{}{}",
        insert_drink_sql(drink, profile, "percent", Some(500.0)),
        insert_component_sql(Uuid::now_v7(), drink, 0, "25"),
        insert_component_sql(Uuid::now_v7(), drink, 1, "75.000001"),
    );
    assert_refused(
        &run_alone(&url, &sql).await,
        CHECK_REFUSED,
        &above_100(drink),
    );
}

#[tokio::test]
async fn components_in_percent_summing_to_100_with_rounding_commit() {
    // 0.2 + 83.9 + 15.9 is 100.00000000000001 in float8 summed in this order;
    // the domain crate accepts it, so must the database.
    let Some(url) = database_url("components_in_percent_summing_to_100_with_rounding_commit")
    else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, _) = drink_with(&url, profile, "percent", Some(200.0), &[0.2, 83.9, 15.9]).await;
    assert!((percent_sum(&pool, drink).await - 100.0).abs() < 1e-9);
}

#[tokio::test]
async fn raising_a_quantity_above_100_in_total_is_refused() {
    let Some(url) = database_url("raising_a_quantity_above_100_in_total_is_refused") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, components) =
        drink_with(&url, profile, "percent", Some(200.0), &[25.0, 75.0]).await;

    let outcome = run_alone(
        &url,
        &format!(
            "UPDATE drink_component SET quantity = 80 WHERE id = '{}'",
            components[1]
        ),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &above_100(drink));
    assert!((percent_sum(&pool, drink).await - 100.0).abs() < 1e-9);
}

#[tokio::test]
async fn moving_a_component_into_a_full_drink_in_percent_is_refused() {
    let Some(url) = database_url("moving_a_component_into_a_full_drink_in_percent_is_refused")
    else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (full, _) = drink_with(&url, profile, "percent", Some(200.0), &[25.0, 75.0]).await;
    let (_, components) = drink_with(&url, profile, "percent", Some(200.0), &[10.0, 20.0]).await;

    let outcome = run_alone(
        &url,
        &format!(
            "UPDATE drink_component SET drink_id = '{full}', sort_order = 2 WHERE id = '{}'",
            components[0]
        ),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &above_100(full));
}

#[tokio::test]
async fn switching_a_drink_to_percent_checks_its_components() {
    // 200 cL + 100 cL read as percentages would be 300 %.
    let Some(url) = database_url("switching_a_drink_to_percent_checks_its_components") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (drink, _) = drink_with(&url, profile, "cl", None, &[200.0, 100.0]).await;

    let outcome = run_alone(
        &url,
        &format!(
            "UPDATE drink SET quantity_unit = 'percent', volume_total_ml = 300 WHERE id = '{drink}'"
        ),
    )
    .await;
    assert_refused(&outcome, CHECK_REFUSED, &above_100(drink));

    // Converted along with the switch, the same drink commits.
    run_alone(
        &url,
        &format!(
            "UPDATE drink SET quantity_unit = 'percent', volume_total_ml = 300 WHERE id = '{drink}';
             UPDATE drink_component SET quantity = quantity / 3 WHERE drink_id = '{drink}'"
        ),
    )
    .await
    .expect("a drink converted to percent commits");
}

// ----- Finite quantities ----------------------------------------------------

#[tokio::test]
async fn a_quantity_or_total_volume_that_is_not_finite_is_refused() {
    // PostgreSQL orders NaN above every number, so `quantity > 0` alone lets
    // both NaN and Infinity through.
    let Some(url) = database_url("a_quantity_or_total_volume_that_is_not_finite_is_refused") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for value in ["NaN", "Infinity"] {
        let drink = Uuid::now_v7();
        let sql = format!(
            "{}{}",
            insert_drink_sql(drink, profile, "cl", None),
            insert_component_sql(Uuid::now_v7(), drink, 0, &format!("'{value}'"))
        );
        assert_refused(
            &run_alone(&url, &sql).await,
            CHECK_VIOLATION,
            "drink_component_quantity_finite",
        );

        let (drink, _) = drink_with(&url, profile, "percent", Some(200.0), &[50.0]).await;
        let outcome = run_alone(
            &url,
            &format!("UPDATE drink SET volume_total_ml = '{value}' WHERE id = '{drink}'"),
        )
        .await;
        assert_refused(&outcome, CHECK_VIOLATION, "drink_volume_total_finite");
    }
}

// ----- Under concurrency ----------------------------------------------------

/// Key of the session advisory lock under which [`in_turn`] runs its pair of
/// transactions, so that no two pairs overlap (see the head of this file).
const PAIRS_LOCK: i64 = 13;

/// Two transactions at the given isolation levels each run their statement;
/// the first one commits, then the second. Both snapshots are taken before
/// either commit, and no other pair runs meanwhile: the advisory lock is held
/// on a third connection until both commits have returned, and released with
/// that connection if the pair panics.
async fn in_turn(
    url: &str,
    (first, first_sql): (&str, &str),
    (last, last_sql): (&str, &str),
) -> (Result<(), sqlx::Error>, Result<(), sqlx::Error>) {
    let mut gate = connection(url).await;
    run(&mut gate, &format!("SELECT pg_advisory_lock({PAIRS_LOCK})"))
        .await
        .expect("the pairs lock must be taken");
    let mut a = connection(url).await;
    let mut b = connection(url).await;
    run(&mut a, &format!("BEGIN ISOLATION LEVEL {first}"))
        .await
        .expect("BEGIN");
    run(&mut b, &format!("BEGIN ISOLATION LEVEL {last}"))
        .await
        .expect("BEGIN");
    run(&mut a, first_sql)
        .await
        .expect("the statement itself must pass: the check is deferred to COMMIT");
    run(&mut b, last_sql)
        .await
        .expect("the statement itself must pass: the check is deferred to COMMIT");
    let outcomes = (run(&mut a, "COMMIT").await, run(&mut b, "COMMIT").await);
    run(
        &mut gate,
        &format!("SELECT pg_advisory_unlock({PAIRS_LOCK})"),
    )
    .await
    .expect("the pairs lock must be released");
    outcomes
}

/// What the last committer must get: under READ COMMITTED it re-reads and is
/// refused by the check itself, under the two snapshot levels it cannot
/// re-read and gets a serialization failure.
fn assert_last_refused(last: &str, outcome: &Result<(), sqlx::Error>, message: &str, case: &str) {
    if last == "READ COMMITTED" {
        assert_refused(outcome, CHECK_REFUSED, message);
    } else {
        let error = outcome
            .as_ref()
            .expect_err(&format!("{case}: the last committer must be refused"));
        assert_eq!(
            sqlstate(error).as_deref(),
            Some(SERIALIZATION_FAILURE),
            "{case}: {error}"
        );
    }
}

#[tokio::test]
async fn two_transactions_cannot_each_remove_one_of_the_last_two_components() {
    // On the initial schema both commit and leave the drink at zero
    // components: each still sees the component the other one removes.
    let Some(url) =
        database_url("two_transactions_cannot_each_remove_one_of_the_last_two_components")
    else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for first in ISOLATION_LEVELS {
        for last in ISOLATION_LEVELS {
            let case = format!("{first} x {last}");
            let (drink, c) = drink_with(&url, profile, "cl", None, &[40.0, 160.0]).await;
            let (first_outcome, last_outcome) = in_turn(
                &url,
                (
                    first,
                    &format!("DELETE FROM drink_component WHERE id = '{}'", c[0]),
                ),
                (
                    last,
                    &format!("DELETE FROM drink_component WHERE id = '{}'", c[1]),
                ),
            )
            .await;
            first_outcome.unwrap_or_else(|e| panic!("{case}: the first committer must pass: {e}"));
            assert_last_refused(last, &last_outcome, &no_component(drink), &case);
            assert_eq!(component_count(&pool, drink).await, 1, "{case}");
        }
    }
}

#[tokio::test]
async fn two_transactions_cannot_each_add_a_share_that_fits_alone() {
    // 40 % already there; each transaction adds 40 %, which fits on its own and
    // makes 120 % together.
    let Some(url) = database_url("two_transactions_cannot_each_add_a_share_that_fits_alone") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for first in ISOLATION_LEVELS {
        for last in ISOLATION_LEVELS {
            let case = format!("{first} x {last}");
            let (drink, _) = drink_with(&url, profile, "percent", Some(200.0), &[40.0]).await;
            let (first_outcome, last_outcome) = in_turn(
                &url,
                (first, &insert_component_sql(Uuid::now_v7(), drink, 1, "40")),
                (last, &insert_component_sql(Uuid::now_v7(), drink, 2, "40")),
            )
            .await;
            first_outcome.unwrap_or_else(|e| panic!("{case}: the first committer must pass: {e}"));
            assert_last_refused(last, &last_outcome, &above_100(drink), &case);
            assert!(
                (percent_sum(&pool, drink).await - 80.0).abs() < 1e-9,
                "{case}"
            );
        }
    }
}

#[tokio::test]
async fn a_unit_switch_and_a_component_insert_cannot_both_commit() {
    // One transaction switches a drink of 60 cL to percent, reading 60 as
    // 60 %; the other adds 50 cL, which a drink in cL takes without a bound.
    let Some(url) = database_url("a_unit_switch_and_a_component_insert_cannot_both_commit") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for first in ISOLATION_LEVELS {
        for last in ISOLATION_LEVELS {
            let case = format!("{first} x {last}");
            let (drink, _) = drink_with(&url, profile, "cl", None, &[60.0]).await;
            let (first_outcome, last_outcome) = in_turn(
                &url,
                (
                    first,
                    &format!(
                        "UPDATE drink SET quantity_unit = 'percent', volume_total_ml = 100
                         WHERE id = '{drink}'"
                    ),
                ),
                (last, &insert_component_sql(Uuid::now_v7(), drink, 1, "50")),
            )
            .await;
            first_outcome.unwrap_or_else(|e| panic!("{case}: the first committer must pass: {e}"));
            assert_last_refused(last, &last_outcome, &above_100(drink), &case);
            assert!(
                (percent_sum(&pool, drink).await - 60.0).abs() < 1e-9,
                "{case}"
            );
        }
    }
}

/// What a client does with a transaction refused by PostgreSQL's serializable
/// snapshot isolation itself: it runs it again, alone. Only when both
/// transactions were `SERIALIZABLE`, the one case where PostgreSQL may report
/// a false conflict, and only for that report (`read/write dependencies`): a
/// `concurrent update` of a guard row is the per-drink serialisation, and stays
/// a failure.
async fn replayed_after_false_conflict(
    url: &str,
    both_serializable: bool,
    (level, sql): (&str, &str),
    outcome: Result<(), sqlx::Error>,
) -> Result<(), sqlx::Error> {
    match outcome {
        Err(error)
            if both_serializable
                && sqlstate(&error).as_deref() == Some(SERIALIZATION_FAILURE)
                && error.to_string().contains("read/write dependencies") =>
        {
            let mut conn = connection(url).await;
            run(&mut conn, &format!("BEGIN ISOLATION LEVEL {level}"))
                .await
                .expect("BEGIN");
            run(&mut conn, sql).await?;
            run(&mut conn, "COMMIT").await
        }
        outcome => outcome,
    }
}

#[tokio::test]
async fn concurrent_writes_on_different_drinks_do_not_interfere() {
    // The serialisation is per drink: emptying one drink's second component
    // while another transaction edits another drink commits both, at every
    // level; when both are SERIALIZABLE, after replaying a transaction refused
    // by a false conflict of PostgreSQL's predicate locks (see the head of
    // this file).
    let Some(url) = database_url("concurrent_writes_on_different_drinks_do_not_interfere") else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;

    for first in ISOLATION_LEVELS {
        for last in ISOLATION_LEVELS {
            let case = format!("{first} x {last}");
            let (one, a) = drink_with(&url, profile, "cl", None, &[40.0, 160.0]).await;
            let (other, b) = drink_with(&url, profile, "cl", None, &[40.0, 160.0]).await;
            let first_sql = format!("DELETE FROM drink_component WHERE id = '{}'", a[0]);
            let last_sql = format!("DELETE FROM drink_component WHERE id = '{}'", b[0]);
            let (first_outcome, last_outcome) =
                in_turn(&url, (first, &first_sql), (last, &last_sql)).await;
            let both_serializable = first == "SERIALIZABLE" && last == "SERIALIZABLE";
            replayed_after_false_conflict(
                &url,
                both_serializable,
                (first, &first_sql),
                first_outcome,
            )
            .await
            .unwrap_or_else(|e| panic!("{case}: first: {e}"));
            replayed_after_false_conflict(&url, both_serializable, (last, &last_sql), last_outcome)
                .await
                .unwrap_or_else(|e| panic!("{case}: last: {e}"));
            assert_eq!(component_count(&pool, one).await, 1, "{case}");
            assert_eq!(component_count(&pool, other).await, 1, "{case}");
        }
    }
}

async fn backend_pid(conn: &mut PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(conn)
        .await
        .expect("the pid must be readable")
}

/// Waits, on the server clock, until every backend of `pids` is blocked on a
/// heavyweight lock. Returns whether they all got there within about 5 s.
async fn wait_until_blocked(pool: &PgPool, pids: &[i32]) -> bool {
    for _ in 0..500 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE pid = ANY($1) AND wait_event_type = 'Lock'",
        )
        .bind(pids)
        .fetch_one(pool)
        .await
        .expect("pg_stat_activity must be readable");
        if usize::try_from(blocked).ok() == Some(pids.len()) {
            return true;
        }
        sqlx::query("SELECT pg_sleep(0.01)")
            .execute(pool)
            .await
            .expect("pg_sleep");
    }
    false
}

#[tokio::test]
async fn two_transactions_writing_two_drinks_in_opposite_orders_do_not_deadlock() {
    // T1 writes drink `low` then `high`, T2 `high` then `low`: their checks
    // fire at COMMIT in that event order. A third connection holds the guard
    // row of `low` until both COMMITs wait behind it. Written in event order,
    // T2 would by then hold `high` and wait for `low`, and T1 would take `low`
    // once released and wait for `high`: a deadlock (40P01). Written in id
    // order, both wait for `low` first and simply queue.
    let name = "two_transactions_writing_two_drinks_in_opposite_orders_do_not_deadlock";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let profile = profile(&pool).await;
    let (one, one_components) = drink_with(&url, profile, "cl", None, &[40.0, 40.0, 40.0]).await;
    let (two, two_components) = drink_with(&url, profile, "cl", None, &[40.0, 40.0, 40.0]).await;
    let ((low, low_c), (high, high_c)) = if one < two {
        ((one, one_components), (two, two_components))
    } else {
        ((two, two_components), (one, one_components))
    };
    let delete = |id: Uuid| format!("DELETE FROM drink_component WHERE id = '{id}';");

    let mut a = connection(&url).await;
    let mut b = connection(&url).await;
    let pids = [backend_pid(&mut a).await, backend_pid(&mut b).await];
    run(&mut a, "BEGIN ISOLATION LEVEL READ COMMITTED")
        .await
        .expect("BEGIN");
    run(&mut b, "BEGIN ISOLATION LEVEL READ COMMITTED")
        .await
        .expect("BEGIN");
    run(
        &mut a,
        &format!("{}{}", delete(low_c[0]), delete(high_c[0])),
    )
    .await
    .expect("T1 writes low then high");
    run(
        &mut b,
        &format!("{}{}", delete(high_c[1]), delete(low_c[1])),
    )
    .await
    .expect("T2 writes high then low");

    let mut holder = connection(&url).await;
    run(&mut holder, "BEGIN").await.expect("BEGIN");
    run(
        &mut holder,
        &format!("SELECT public.write_drink_component_guard('{low}')"),
    )
    .await
    .expect("the holder must write the guard row");

    let commit_a = tokio::spawn(async move { run(&mut a, "COMMIT").await });
    let commit_b = tokio::spawn(async move { run(&mut b, "COMMIT").await });
    let both_blocked = wait_until_blocked(&pool, &pids).await;
    run(&mut holder, "COMMIT")
        .await
        .expect("the holder commits");
    let outcomes = [
        commit_a.await.expect("task a"),
        commit_b.await.expect("task b"),
    ];

    assert!(both_blocked, "both COMMITs must queue behind the holder");
    for (who, outcome) in ["T1", "T2"].iter().zip(&outcomes) {
        if let Err(error) = outcome {
            panic!(
                "{who} must commit, got SQLSTATE {:?}: {error}",
                sqlstate(error)
            );
        }
    }
    assert_eq!(component_count(&pool, low).await, 1);
    assert_eq!(component_count(&pool, high).await, 1);
}

#[tokio::test]
async fn the_seed_stores_cl_quantities_in_centilitres() {
    // `db::seed` refuses a database that holds profiles, so it runs on a
    // database of its own, created on the same server and dropped afterwards.
    let Some(url) = database_url("the_seed_stores_cl_quantities_in_centilitres") else {
        return;
    };
    let (server, _) = url
        .rsplit_once('/')
        .expect("DATABASE_URL must name a database");
    let name = format!("seed_{}", Uuid::now_v7().simple());
    let mut admin = connection(&url).await;
    run(&mut admin, &format!("CREATE DATABASE {name}"))
        .await
        .expect("the seed database must be created");

    let pool = migrated_pool(&format!("{server}/{name}")).await;
    let seeded = db::seed(&pool).await;
    let rows: Result<Vec<(String, String, f64)>, sqlx::Error> = sqlx::query_as(
        "SELECT c.name, d.quantity_unit::text, c.quantity
           FROM drink_component c JOIN drink d ON d.id = c.drink_id
          ORDER BY c.name",
    )
    .fetch_all(&pool)
    .await;
    pool.close().await;
    run(&mut admin, &format!("DROP DATABASE {name} WITH (FORCE)"))
        .await
        .expect("the seed database must be dropped");

    seeded.expect("the seed must apply to an empty database");
    // A drink in cl stores centilitres, as the API exchanges them (#14): the
    // beer is 25 cL, i.e. 250 mL, and the vodka 4 cL, i.e. 40 mL.
    let expected = [
        ("Bière blonde", "cl", 25.0),
        ("Gin", "percent", 25.0),
        ("Jus d'orange", "percent", 75.0),
        ("Vodka", "cl", 4.0),
    ]
    .map(|(name, unit, quantity)| (name.to_owned(), unit.to_owned(), quantity));
    assert_eq!(rows.expect("the seeded components must read"), expected);
}
