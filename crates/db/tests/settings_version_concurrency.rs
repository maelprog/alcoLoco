//! "Every profile has at least one settings version" under concurrent writes
//! (issue #43, SPEC.md §5.1 and §10.0-L).
//!
//! Each test drives two or three real connections by hand, so it needs a real
//! PostgreSQL. Like `crates/api/tests/database.rs`, every test is conditioned on
//! `DATABASE_URL` and skips itself when it is not set, and
//! `ALCOLOCO_REQUIRE_DB=1` turns that skip into a failure:
//!
//! ```text
//! ALCOLOCO_REQUIRE_DB=1 DATABASE_URL=postgres://… cargo test -p db --test settings_version_concurrency
//! ```
//!
//! None of these tests depends on timing. The races they reproduce are the ones
//! a fixed commit order already decides — under `REPEATABLE READ` the second
//! committer reads through a snapshot taken before the first one committed, at
//! whatever distance the two commits are — or ones whose interleaving is forced
//! by a third connection holding a lock. The width of the `READ COMMITTED`
//! window, which does depend on timing, is measured outside of the test suite
//! with both commits synchronised on the server clock; see the pull request
//! that closed #43.

use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};
use sqlx::{Connection, Executor};
use uuid::Uuid;

/// Set to a non-empty value other than `0`, turns every skip into a failure.
const REQUIRE_DATABASE_ENV: &str = "ALCOLOCO_REQUIRE_DB";

/// SQLSTATE of the check's own refusal (`integrity_constraint_violation`).
const CHECK_REFUSED: &str = "23000";

/// SQLSTATE `serialization_failure`.
const SERIALIZATION_FAILURE: &str = "40001";

/// SQLSTATE `deadlock_detected`.
const DEADLOCK_DETECTED: &str = "40P01";

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

/// Inserts a profile owning `versions` settings versions, one year apart from
/// 2020, and returns the profile id followed by the version ids in `valid_from`
/// order.
async fn profile_with_versions(pool: &PgPool, versions: usize) -> (Uuid, Vec<Uuid>) {
    profile_with_versions_from(pool, versions, 0).await
}

/// Same, the first version starting `first_year` years after 2020. A version
/// moved onto another profile must not share its `valid_from` with a version
/// already there: `profile_settings_version_unique_start` would reject the move
/// on the spot, before the trigger under test is ever reached.
async fn profile_with_versions_from(
    pool: &PgPool,
    versions: usize,
    first_year: usize,
) -> (Uuid, Vec<Uuid>) {
    let profile = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("a transaction must open");
    sqlx::query(
        "INSERT INTO profile (
             id, display_name, default_quantity_unit,
             default_ingestion_duration_seconds, default_absorption_duration_seconds
         ) VALUES ($1, 'concurrency', 'cl', 60, 60)",
    )
    .bind(profile)
    .execute(&mut *tx)
    .await
    .expect("the profile must insert");

    let mut ids = Vec::with_capacity(versions);
    for year in first_year..first_year + versions {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO profile_settings_version (
                 id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
             ) VALUES ($1, $2, timestamptz '2020-01-01' + make_interval(years => $3),
                       70, 170, 'male', date '1990-01-01')",
        )
        .bind(id)
        .bind(profile)
        .bind(i32::try_from(year).expect("a handful of versions"))
        .execute(&mut *tx)
        .await
        .expect("the version must insert");
        ids.push(id);
    }
    tx.commit().await.expect("the fixture must commit");
    (profile, ids)
}

async fn version_count(pool: &PgPool, profile: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM profile_settings_version WHERE profile_id = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}

async fn run(conn: &mut PgConnection, sql: &str) -> Result<(), sqlx::Error> {
    conn.execute(sql).await.map(|_| ())
}

async fn delete_version(conn: &mut PgConnection, version: Uuid) {
    run(
        conn,
        &format!("DELETE FROM profile_settings_version WHERE id = '{version}'"),
    )
    .await
    .expect("the DELETE itself must pass: the check is deferred to COMMIT");
}

async fn move_version(conn: &mut PgConnection, version: Uuid, to: Uuid) {
    run(
        conn,
        &format!("UPDATE profile_settings_version SET profile_id = '{to}' WHERE id = '{version}'"),
    )
    .await
    .expect("the UPDATE itself must pass: the check is deferred to COMMIT");
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .map(|code| code.into_owned())
}

/// Asserts that `outcome` is the refusal of the check itself: its SQLSTATE and
/// its message, not a serialization failure or a lock error that happens to
/// leave the database in the right state.
fn assert_refused_by_the_check(outcome: &Result<(), sqlx::Error>, profile: Uuid) {
    let error = outcome
        .as_ref()
        .expect_err("the COMMIT must be refused by the check");
    assert_eq!(
        sqlstate(error).as_deref(),
        Some(CHECK_REFUSED),
        "wrong mechanism: {error}"
    );
    assert!(
        error
            .to_string()
            .contains(&format!("profile {profile} has no settings version")),
        "wrong message: {error}"
    );
}

#[tokio::test]
async fn deleting_every_version_in_one_transaction_is_refused_by_the_check() {
    // The negative control of every concurrency measurement: without it, a
    // scenario that "holds" could simply be one the check never looks at.
    let name = "deleting_every_version_in_one_transaction_is_refused_by_the_check";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, versions) = profile_with_versions(&pool, 2).await;

    for isolation in ["READ COMMITTED", "REPEATABLE READ", "SERIALIZABLE"] {
        let mut conn = connection(&url).await;
        run(&mut conn, &format!("BEGIN ISOLATION LEVEL {isolation}"))
            .await
            .expect("BEGIN");
        delete_version(&mut conn, versions[0]).await;
        delete_version(&mut conn, versions[1]).await;
        assert_refused_by_the_check(&run(&mut conn, "COMMIT").await, profile);
    }

    assert_eq!(version_count(&pool, profile).await, 2);
}

/// Two transactions at the given isolation levels each delete one of the two
/// versions of a profile; the first one commits, then the second. Both
/// snapshots are taken before either commit.
async fn delete_both_versions_in_turn(
    url: &str,
    pool: &PgPool,
    first: &str,
    last: &str,
) -> (Uuid, Result<(), sqlx::Error>, Result<(), sqlx::Error>) {
    let (profile, versions) = profile_with_versions(pool, 2).await;
    let mut a = connection(url).await;
    let mut b = connection(url).await;

    run(&mut a, &format!("BEGIN ISOLATION LEVEL {first}"))
        .await
        .expect("BEGIN");
    run(&mut b, &format!("BEGIN ISOLATION LEVEL {last}"))
        .await
        .expect("BEGIN");
    delete_version(&mut a, versions[0]).await;
    delete_version(&mut b, versions[1]).await;

    let first_outcome = run(&mut a, "COMMIT").await;
    let last_outcome = run(&mut b, "COMMIT").await;
    (profile, first_outcome, last_outcome)
}

#[tokio::test]
async fn a_repeatable_read_last_committer_cannot_empty_a_profile() {
    // Under REPEATABLE READ the deferred check reads through the snapshot of
    // its transaction, taken before the other one committed: it still sees the
    // version the other one removed, at any distance between the two commits.
    // The first committer can be at any level; the last one is refused.
    let name = "a_repeatable_read_last_committer_cannot_empty_a_profile";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;

    for first in ["READ COMMITTED", "REPEATABLE READ", "SERIALIZABLE"] {
        for last in ["REPEATABLE READ", "SERIALIZABLE"] {
            let (profile, first_outcome, last_outcome) =
                delete_both_versions_in_turn(&url, &pool, first, last).await;
            first_outcome.unwrap_or_else(|error| {
                panic!("{first} x {last}: the first committer must pass: {error}")
            });
            let error = last_outcome.expect_err(&format!(
                "{first} x {last}: the last committer must be refused"
            ));
            assert_eq!(
                sqlstate(&error).as_deref(),
                Some(SERIALIZATION_FAILURE),
                "{first} x {last}: {error}"
            );
            assert_eq!(
                version_count(&pool, profile).await,
                1,
                "{first} x {last}: the profile must keep its last version"
            );
        }
    }
}

#[tokio::test]
async fn a_read_committed_last_committer_is_refused_by_the_check() {
    // Committed one after the other, a READ COMMITTED last committer sees the
    // first commit and is refused by the check itself. This already held
    // before #43; it is kept to prove the fix did not trade the check's own
    // error for another one.
    let name = "a_read_committed_last_committer_is_refused_by_the_check";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;

    for first in ["READ COMMITTED", "REPEATABLE READ", "SERIALIZABLE"] {
        let (profile, first_outcome, last_outcome) =
            delete_both_versions_in_turn(&url, &pool, first, "READ COMMITTED").await;
        first_outcome
            .unwrap_or_else(|error| panic!("{first}: the first committer must pass: {error}"));
        assert_refused_by_the_check(&last_outcome, profile);
        assert_eq!(version_count(&pool, profile).await, 1, "{first}");
    }
}

#[tokio::test]
async fn moving_a_version_away_is_covered_like_deleting_it() {
    // `UPDATE ... SET profile_id` takes a version away from its profile just as
    // a DELETE does, and goes through the same trigger.
    let name = "moving_a_version_away_is_covered_like_deleting_it";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;

    for isolation in ["REPEATABLE READ", "READ COMMITTED"] {
        let (profile, versions) = profile_with_versions(&pool, 2).await;
        let (sink, _) = profile_with_versions_from(&pool, 1, 10).await;
        let mut a = connection(&url).await;
        let mut b = connection(&url).await;

        run(&mut a, &format!("BEGIN ISOLATION LEVEL {isolation}"))
            .await
            .expect("BEGIN");
        run(&mut b, &format!("BEGIN ISOLATION LEVEL {isolation}"))
            .await
            .expect("BEGIN");
        move_version(&mut a, versions[0], sink).await;
        delete_version(&mut b, versions[1]).await;

        run(&mut a, "COMMIT")
            .await
            .unwrap_or_else(|error| panic!("{isolation}: the move must commit: {error}"));
        let outcome = run(&mut b, "COMMIT").await;
        if isolation == "READ COMMITTED" {
            assert_refused_by_the_check(&outcome, profile);
        } else {
            let error = outcome.expect_err("the last committer must be refused");
            assert_eq!(sqlstate(&error).as_deref(), Some(SERIALIZATION_FAILURE));
        }
        assert_eq!(version_count(&pool, profile).await, 1, "{isolation}");
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
async fn read_committed_commits_that_overlap_leave_the_last_version_in_place() {
    // The race of #43 at its narrowest: two READ COMMITTED transactions whose
    // commits overlap. The overlap is forced rather than hoped for: a third
    // connection holds the profile row, both COMMITs are sent, and the row is
    // released only once both are seen waiting behind it. Exactly one of them
    // may pass, and the other must be refused by the check itself.
    let name = "read_committed_commits_that_overlap_leave_the_last_version_in_place";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, versions) = profile_with_versions(&pool, 2).await;

    let mut holder = connection(&url).await;
    let mut a = connection(&url).await;
    let mut b = connection(&url).await;
    let pids = [backend_pid(&mut a).await, backend_pid(&mut b).await];

    run(&mut a, "BEGIN ISOLATION LEVEL READ COMMITTED")
        .await
        .expect("BEGIN");
    run(&mut b, "BEGIN ISOLATION LEVEL READ COMMITTED")
        .await
        .expect("BEGIN");
    delete_version(&mut a, versions[0]).await;
    delete_version(&mut b, versions[1]).await;

    run(&mut holder, "BEGIN").await.expect("BEGIN");
    run(
        &mut holder,
        &format!("SELECT 1 FROM profile WHERE id = '{profile}' FOR NO KEY UPDATE"),
    )
    .await
    .expect("the holder must lock the profile row");

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

    assert!(
        both_blocked,
        "both COMMITs must queue on the profile row: the check has to serialise \
         the writers of one profile at commit time"
    );
    let refused: Vec<_> = outcomes.iter().filter(|outcome| outcome.is_err()).collect();
    assert_eq!(
        refused.len(),
        1,
        "exactly one COMMIT must be refused: {outcomes:?}"
    );
    assert_refused_by_the_check(refused[0], profile);
    assert_eq!(version_count(&pool, profile).await, 1);
}

#[tokio::test]
async fn opposite_traversal_orders_do_not_deadlock_at_commit() {
    // Two transactions remove one version from each of two profiles, in
    // opposite orders — the probe that condemned `SELECT ... FOR UPDATE` in the
    // check (#43). The rows each check has to lock at COMMIT are then the same
    // two profiles, reached in opposite orders if they were locked in the order
    // of the row events. A third connection holds both profile rows until both
    // COMMITs are queued, so that both resume at once: locked in event order
    // they deadlock, locked in one global order they do not.
    let name = "opposite_traversal_orders_do_not_deadlock_at_commit";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;

    for round in 0..5 {
        let (x, xs) = profile_with_versions(&pool, 3).await;
        let (y, ys) = profile_with_versions(&pool, 3).await;

        let mut holder = connection(&url).await;
        let mut a = connection(&url).await;
        let mut b = connection(&url).await;
        let pids = [backend_pid(&mut a).await, backend_pid(&mut b).await];

        run(&mut a, "BEGIN").await.expect("BEGIN");
        run(&mut b, "BEGIN").await.expect("BEGIN");
        delete_version(&mut a, xs[0]).await;
        delete_version(&mut a, ys[0]).await;
        delete_version(&mut b, ys[1]).await;
        delete_version(&mut b, xs[1]).await;

        run(&mut holder, "BEGIN").await.expect("BEGIN");
        run(
            &mut holder,
            &format!("SELECT 1 FROM profile WHERE id IN ('{x}', '{y}') FOR NO KEY UPDATE"),
        )
        .await
        .expect("the holder must lock both profile rows");

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

        assert!(both_blocked, "round {round}: both COMMITs must queue");
        for outcome in &outcomes {
            if let Err(error) = outcome {
                assert_ne!(
                    sqlstate(error).as_deref(),
                    Some(DEADLOCK_DETECTED),
                    "round {round}: deadlock at COMMIT: {error}"
                );
                panic!("round {round}: no COMMIT may fail here: {error}");
            }
        }
        assert_eq!(version_count(&pool, x).await, 1, "round {round}");
        assert_eq!(version_count(&pool, y).await, 1, "round {round}");
    }
}

#[tokio::test]
async fn deleting_a_profile_while_one_of_its_versions_is_removed_does_not_deadlock() {
    // A transaction has removed one version of a profile and not committed yet;
    // another deletes the profile, which cascades to its versions. The check
    // writes the profile row at COMMIT, so the order in which the two meet
    // matters: if the profile DELETE got hold of the row first, it would wait
    // for the version the first transaction removed while that transaction
    // waits for the row. The removal takes the lock a foreign key check takes,
    // `FOR KEY SHARE` on the profile, so the DELETE queues up front instead.
    let name = "deleting_a_profile_while_one_of_its_versions_is_removed_does_not_deadlock";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, versions) = profile_with_versions(&pool, 2).await;

    let mut remover = connection(&url).await;
    let mut deleter = connection(&url).await;
    let deleter_pid = backend_pid(&mut deleter).await;

    run(&mut remover, "BEGIN").await.expect("BEGIN");
    delete_version(&mut remover, versions[0]).await;

    let delete_profile = tokio::spawn(async move {
        run(
            &mut deleter,
            &format!("DELETE FROM profile WHERE id = '{profile}'"),
        )
        .await
    });
    let deleter_blocked = wait_until_blocked(&pool, &[deleter_pid]).await;
    let committed = run(&mut remover, "COMMIT").await;
    let deleted = delete_profile.await.expect("task");

    assert!(
        deleter_blocked,
        "the profile DELETE must wait for the remover"
    );
    for outcome in [&committed, &deleted] {
        if let Err(error) = outcome {
            panic!("no statement may fail here: {error}");
        }
    }
    assert_eq!(version_count(&pool, profile).await, 0);
}
