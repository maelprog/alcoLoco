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

/// Opens a transaction on a fresh connection that writes the guard row of each
/// of `profiles` and keeps it: every check of those profiles then queues behind
/// this connection until it commits.
async fn hold_guard_rows(url: &str, profiles: &[Uuid]) -> PgConnection {
    let mut holder = connection(url).await;
    run(&mut holder, "BEGIN").await.expect("BEGIN");
    for profile in profiles {
        run(
            &mut holder,
            &format!("SELECT public.write_profile_settings_guard('{profile}')"),
        )
        .await
        .expect("the holder must write the guard row");
    }
    holder
}

#[tokio::test]
async fn read_committed_commits_that_overlap_leave_the_last_version_in_place() {
    // The race of #43 at its narrowest: two READ COMMITTED transactions whose
    // commits overlap. The overlap is forced rather than hoped for: a third
    // connection holds the guard row of the profile, both COMMITs are sent, and
    // the row is released only once both are seen waiting behind it. Exactly
    // one of them may pass, and the other must be refused by the check itself.
    let name = "read_committed_commits_that_overlap_leave_the_last_version_in_place";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, versions) = profile_with_versions(&pool, 2).await;

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

    let mut holder = hold_guard_rows(&url, &[profile]).await;
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
        "both COMMITs must queue on the guard row: the check has to serialise \
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
    // check (#43). The guard rows each check has to write at COMMIT are then
    // those of the same two profiles, reached in opposite orders if they were
    // written in the order of the row events. A third connection holds both
    // guard rows until both COMMITs are queued, so that both resume at once:
    // written in event order they deadlock, in one global order they do not.
    let name = "opposite_traversal_orders_do_not_deadlock_at_commit";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;

    for round in 0..5 {
        let (x, xs) = profile_with_versions(&pool, 3).await;
        let (y, ys) = profile_with_versions(&pool, 3).await;

        let mut a = connection(&url).await;
        let mut b = connection(&url).await;
        let pids = [backend_pid(&mut a).await, backend_pid(&mut b).await];

        run(&mut a, "BEGIN").await.expect("BEGIN");
        run(&mut b, "BEGIN").await.expect("BEGIN");
        delete_version(&mut a, xs[0]).await;
        delete_version(&mut a, ys[0]).await;
        delete_version(&mut b, ys[1]).await;
        delete_version(&mut b, xs[1]).await;

        let mut holder = hold_guard_rows(&url, &[x, y]).await;
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

/// Runs `statements` in order on `conn`, stopping at the first error, after
/// which the transaction is rolled back. Returns the connection with the
/// outcome, so that a task can own the connection and hand it back.
async fn run_all(
    mut conn: PgConnection,
    statements: Vec<String>,
) -> (PgConnection, Result<(), sqlx::Error>) {
    for statement in statements {
        if let Err(error) = run(&mut conn, &statement).await {
            let _ = run(&mut conn, "ROLLBACK").await;
            return (conn, Err(error));
        }
    }
    (conn, Ok(()))
}

type Session = tokio::task::JoinHandle<(PgConnection, Result<(), sqlx::Error>)>;

fn spawn_session(conn: PgConnection, statements: &[String]) -> Session {
    tokio::spawn(run_all(conn, statements.to_vec()))
}

/// Waits until the task driving backend `pid` has either finished or is
/// blocked on a heavyweight lock, whichever the schema makes it do.
async fn wait_until_settled(pool: &PgPool, pid: i32, task: &Session) {
    for _ in 0..500 {
        if task.is_finished() || wait_until_blocked_once(pool, pid).await {
            return;
        }
        sqlx::query("SELECT pg_sleep(0.01)")
            .execute(pool)
            .await
            .expect("pg_sleep");
    }
    panic!("backend {pid} neither finished nor blocked within 5 s");
}

async fn wait_until_blocked_once(pool: &PgPool, pid: i32) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                        WHERE pid = $1 AND wait_event_type = 'Lock')",
    )
    .bind(pid)
    .fetch_one(pool)
    .await
    .expect("pg_stat_activity must be readable")
}

async fn outcome_of(session: Session) -> Result<(), sqlx::Error> {
    session.await.expect("the session task must not panic").1
}

fn delete_sql(version: Uuid) -> String {
    format!("DELETE FROM profile_settings_version WHERE id = '{version}'")
}

/// Six interleavings of ordinary DML around the removal of a settings version,
/// each of which commits without error on the initial schema. The check must
/// not turn any of them into a deadlock: it may only wait for other checks.
#[tokio::test]
async fn ordinary_dml_around_a_version_removal_does_not_deadlock() {
    let name = "ordinary_dml_around_a_version_removal_does_not_deadlock";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let s = |text: &str| text.to_owned();

    // 1. Crossed: t renames X then removes a version of Y; t2 renames Y then
    //    removes a version of X. Both COMMITs are sent together.
    {
        let (x, xs) = profile_with_versions(&pool, 3).await;
        let (y, ys) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let mut t2 = connection(&url).await;
        for (conn, renamed, removed) in [(&mut t, x, ys[0]), (&mut t2, y, xs[0])] {
            run(conn, "BEGIN").await.expect("BEGIN");
            run(
                conn,
                &format!("UPDATE profile SET display_name = 'renamed' WHERE id = '{renamed}'"),
            )
            .await
            .expect("rename");
            run(conn, &delete_sql(removed)).await.expect("remove");
        }
        let commits = [
            spawn_session(t, &[s("COMMIT")]),
            spawn_session(t2, &[s("COMMIT")]),
        ];
        for commit in commits {
            outcome_of(commit)
                .await
                .unwrap_or_else(|error| panic!("crossed: {error}"));
        }
    }

    // 2. One profile: t removes v1 of P; t2 renames P, then removes the same
    //    v1, which waits for t; then t commits.
    {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let mut t2 = connection(&url).await;
        let t2_pid = backend_pid(&mut t2).await;
        run(&mut t, "BEGIN").await.expect("BEGIN");
        run(&mut t, &delete_sql(ps[0])).await.expect("remove");
        run(&mut t2, "BEGIN").await.expect("BEGIN");
        run(
            &mut t2,
            &format!("UPDATE profile SET display_name = 'renamed' WHERE id = '{p}'"),
        )
        .await
        .expect("rename");
        let second = spawn_session(t2, &[delete_sql(ps[0]), s("COMMIT")]);
        assert!(
            wait_until_blocked(&pool, &[t2_pid]).await,
            "the second removal must wait for the first"
        );
        run(&mut t, "COMMIT")
            .await
            .unwrap_or_else(|error| panic!("rename then remove, t: {error}"));
        outcome_of(second)
            .await
            .unwrap_or_else(|error| panic!("rename then remove, t2: {error}"));
        assert_eq!(version_count(&pool, p).await, 2);
    }

    // 3 and 4. t2 locks P (FOR UPDATE, or by renaming it); t removes v1 of P
    //    and commits; t2 deletes P.
    for (label, lock) in [
        (
            "lock then delete",
            "SELECT 1 FROM profile WHERE id = '{p}' FOR UPDATE",
        ),
        (
            "rename then delete",
            "UPDATE profile SET display_name = 'renamed' WHERE id = '{p}'",
        ),
    ] {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let t_pid = backend_pid(&mut t).await;
        let mut t2 = connection(&url).await;
        run(&mut t2, "BEGIN").await.expect("BEGIN");
        run(&mut t2, &lock.replace("{p}", &p.to_string()))
            .await
            .expect("lock");
        let remover = spawn_session(t, &[s("BEGIN"), delete_sql(ps[0]), s("COMMIT")]);
        wait_until_settled(&pool, t_pid, &remover).await;
        let deleter = spawn_session(
            t2,
            &[format!("DELETE FROM profile WHERE id = '{p}'"), s("COMMIT")],
        );
        outcome_of(remover)
            .await
            .unwrap_or_else(|error| panic!("{label}, remover: {error}"));
        outcome_of(deleter)
            .await
            .unwrap_or_else(|error| panic!("{label}, deleter: {error}"));
        assert_eq!(version_count(&pool, p).await, 0, "{label}");
    }

    // 5. Both share-lock P and remove a different version each; both COMMITs
    //    are sent together.
    {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let mut t2 = connection(&url).await;
        for (conn, removed) in [(&mut t, ps[0]), (&mut t2, ps[1])] {
            run(conn, "BEGIN").await.expect("BEGIN");
            run(
                conn,
                &format!("SELECT 1 FROM profile WHERE id = '{p}' FOR SHARE"),
            )
            .await
            .expect("share lock");
            run(conn, &delete_sql(removed)).await.expect("remove");
        }
        let commits = [
            spawn_session(t, &[s("COMMIT")]),
            spawn_session(t2, &[s("COMMIT")]),
        ];
        for commit in commits {
            outcome_of(commit)
                .await
                .unwrap_or_else(|error| panic!("share both: {error}"));
        }
        assert_eq!(version_count(&pool, p).await, 1);
    }

    // 6. A DELETE FROM profile over two rows has deleted P and waits on the
    //    second one, X, held by a third connection: P's cascade has not run
    //    yet. t removes v1 of P meanwhile and commits; then X is released.
    {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let (x, _) = profile_with_versions(&pool, 3).await;
        let mut holder = connection(&url).await;
        run(&mut holder, "BEGIN").await.expect("BEGIN");
        run(
            &mut holder,
            &format!("SELECT 1 FROM profile WHERE id = '{x}' FOR UPDATE"),
        )
        .await
        .expect("hold X");
        let mut t2 = connection(&url).await;
        let t2_pid = backend_pid(&mut t2).await;
        let deleter = spawn_session(
            t2,
            &[
                format!("DELETE FROM profile WHERE id IN ('{p}', '{x}')"),
                s("COMMIT"),
            ],
        );
        assert!(
            wait_until_blocked(&pool, &[t2_pid]).await,
            "the multi-row DELETE must wait for X"
        );
        let mut t = connection(&url).await;
        let t_pid = backend_pid(&mut t).await;
        let remover = spawn_session(t, &[s("BEGIN"), delete_sql(ps[0]), s("COMMIT")]);
        wait_until_settled(&pool, t_pid, &remover).await;
        run(&mut holder, "COMMIT").await.expect("release X");
        outcome_of(remover)
            .await
            .unwrap_or_else(|error| panic!("multi-row delete, remover: {error}"));
        outcome_of(deleter)
            .await
            .unwrap_or_else(|error| panic!("multi-row delete, deleter: {error}"));
        assert_eq!(version_count(&pool, p).await, 0);
    }
}

/// SQLSTATE `insufficient_privilege`.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

#[tokio::test]
async fn removing_a_version_needs_insert_update_and_select_on_the_guard_table() {
    // The trigger functions run with the privileges of the caller, so the
    // migration states which ones a role removing versions needs on
    // `profile_settings_guard`. SELECT is the easy one to miss: the check's
    // `ON CONFLICT DO UPDATE` reads the existing row. Each privilege is taken
    // away in turn, and the refusal must come from the privilege check at
    // COMMIT, the profile keeping both of its versions.
    let name = "removing_a_version_needs_insert_update_and_select_on_the_guard_table";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let mut admin = connection(&url).await;

    for missing in [None, Some("INSERT"), Some("UPDATE"), Some("SELECT")] {
        let role = format!("guard_probe_{}", Uuid::now_v7().simple());
        let guard_privileges = ["INSERT", "UPDATE", "SELECT"]
            .into_iter()
            .filter(|privilege| Some(*privilege) != missing)
            .collect::<Vec<_>>()
            .join(", ");
        for grant in [
            format!("CREATE ROLE {role}"),
            format!("GRANT SELECT, DELETE ON profile_settings_version TO {role}"),
            format!("GRANT SELECT ON profile TO {role}"),
            format!("GRANT {guard_privileges} ON profile_settings_guard TO {role}"),
        ] {
            run(&mut admin, &grant)
                .await
                .unwrap_or_else(|error| panic!("{grant}: {error}"));
        }

        let (profile, versions) = profile_with_versions(&pool, 2).await;
        let mut conn = connection(&url).await;
        run(&mut conn, "BEGIN").await.expect("BEGIN");
        run(&mut conn, &format!("SET LOCAL ROLE {role}"))
            .await
            .expect("SET ROLE");
        delete_version(&mut conn, versions[0]).await;
        let outcome = run(&mut conn, "COMMIT").await;
        drop(conn);

        for cleanup in [format!("DROP OWNED BY {role}"), format!("DROP ROLE {role}")] {
            run(&mut admin, &cleanup)
                .await
                .unwrap_or_else(|error| panic!("{cleanup}: {error}"));
        }

        match missing {
            None => {
                outcome.unwrap_or_else(|error| {
                    panic!("INSERT, UPDATE and SELECT must be enough: {error}")
                });
                assert_eq!(version_count(&pool, profile).await, 1);
            }
            Some(privilege) => {
                let error = outcome.expect_err("the COMMIT must be refused");
                assert_eq!(
                    sqlstate(&error).as_deref(),
                    Some(INSUFFICIENT_PRIVILEGE),
                    "without {privilege}: wrong mechanism: {error}"
                );
                assert_eq!(version_count(&pool, profile).await, 2);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The guard row of a deleted profile (#75)
// ---------------------------------------------------------------------------

/// SQLSTATE `foreign_key_violation`.
const FOREIGN_KEY_VIOLATION: &str = "23503";

async fn guard_rows(pool: &PgPool, profile: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM profile_settings_guard WHERE profile_id = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("the count must run")
}

#[tokio::test]
async fn a_guard_row_is_written_with_its_profile_and_deleted_with_it() {
    // Since migration 20261025090000 the guard row of a profile exists from
    // the creation of the profile on, and goes with it (`ON DELETE CASCADE`
    // towards `profile`): a deleted profile leaves no guard row behind, and
    // none can be written for a profile that does not exist.
    let name = "a_guard_row_is_written_with_its_profile_and_deleted_with_it";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, _) = profile_with_versions(&pool, 2).await;
    assert_eq!(guard_rows(&pool, profile).await, 1, "written at creation");

    let mut conn = connection(&url).await;
    run(
        &mut conn,
        &format!("DELETE FROM profile WHERE id = '{profile}'"),
    )
    .await
    .expect("a profile deletes with its versions and its guard row");
    assert_eq!(version_count(&pool, profile).await, 0);
    assert_eq!(guard_rows(&pool, profile).await, 0, "deleted with it");

    let orphan = run(
        &mut conn,
        &format!("INSERT INTO profile_settings_guard (profile_id) VALUES ('{profile}')"),
    )
    .await
    .expect_err("a guard row needs its profile");
    assert_eq!(
        sqlstate(&orphan).as_deref(),
        Some(FOREIGN_KEY_VIOLATION),
        "wrong mechanism: {orphan}"
    );
}

#[tokio::test]
async fn removing_versions_then_the_profile_in_one_transaction_commits() {
    // The removal of a version records its profile for the ordered pass of
    // the check, which runs at COMMIT — after the profile, and its guard row
    // with it, are gone. That pass must skip the profile rather than write a
    // guard row the foreign key refuses.
    let name = "removing_versions_then_the_profile_in_one_transaction_commits";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let (profile, versions) = profile_with_versions(&pool, 2).await;

    let mut conn = connection(&url).await;
    run(&mut conn, "BEGIN").await.expect("BEGIN");
    delete_version(&mut conn, versions[0]).await;
    run(
        &mut conn,
        &format!("DELETE FROM profile WHERE id = '{profile}'"),
    )
    .await
    .expect("the profile must delete");
    run(&mut conn, "COMMIT")
        .await
        .unwrap_or_else(|error| panic!("the COMMIT must pass: {error}"));
    assert_eq!(version_count(&pool, profile).await, 0);
    assert_eq!(guard_rows(&pool, profile).await, 0);
}

/// A statement adding a settings version to `profile`, in 2030: after every
/// version [`profile_with_versions`] writes.
fn insert_version_sql(profile: Uuid) -> String {
    format!(
        "INSERT INTO profile_settings_version (
             id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
         ) VALUES ('{}', '{profile}', timestamptz '2030-01-01', 80, 170, 'male',
                   date '1990-01-01')",
        Uuid::now_v7()
    )
}

#[tokio::test]
async fn deleting_a_profile_while_its_settings_change_does_not_deadlock() {
    // Plain DML under READ COMMITTED, the level of the application: the
    // deletion of a profile races the removal or the addition of one of its
    // settings versions, in both orders. Each transaction either commits or
    // is refused on the spot by a foreign key — never a deadlock, never a
    // version or a guard row left for a profile that is gone.
    let name = "deleting_a_profile_while_its_settings_change_does_not_deadlock";
    let Some(url) = database_url(name) else {
        return;
    };
    let pool = migrated_pool(&url).await;
    let s = |text: &str| text.to_owned();
    let delete_profile = |p: Uuid| format!("DELETE FROM profile WHERE id = '{p}'");

    // 1 and 2. t changes the settings of P and has not committed; t2 deletes
    //    P and waits for t; t commits, then t2. In the removal case the
    //    deletion waits in its cascade towards the versions, and must not be
    //    holding the guard row t writes at COMMIT by then.
    for (label, removal) in [
        ("change first, removal", true),
        ("change first, addition", false),
    ] {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let mut t2 = connection(&url).await;
        let t2_pid = backend_pid(&mut t2).await;
        run(&mut t, "BEGIN").await.expect("BEGIN");
        let change = if removal {
            delete_sql(ps[0])
        } else {
            insert_version_sql(p)
        };
        run(&mut t, &change).await.expect("the change itself");
        let deleter = spawn_session(t2, &[s("BEGIN"), delete_profile(p), s("COMMIT")]);
        assert!(
            wait_until_blocked(&pool, &[t2_pid]).await,
            "{label}: the deletion must wait for the change"
        );
        run(&mut t, "COMMIT")
            .await
            .unwrap_or_else(|error| panic!("{label}, change: {error}"));
        outcome_of(deleter)
            .await
            .unwrap_or_else(|error| panic!("{label}, deletion: {error}"));
        assert_eq!(version_count(&pool, p).await, 0, "{label}");
        assert_eq!(guard_rows(&pool, p).await, 0, "{label}");
    }

    // 3 and 4. t2 has deleted P and has not committed; t changes the settings
    //    of P and waits for t2; t2 commits. The removal then finds nothing to
    //    remove and commits; the addition is refused by the foreign key of
    //    the version, P being gone.
    for (label, removal) in [
        ("deletion first, removal", true),
        ("deletion first, addition", false),
    ] {
        let (p, ps) = profile_with_versions(&pool, 3).await;
        let mut t = connection(&url).await;
        let t_pid = backend_pid(&mut t).await;
        let mut t2 = connection(&url).await;
        run(&mut t2, "BEGIN").await.expect("BEGIN");
        run(&mut t2, &delete_profile(p))
            .await
            .expect("the deletion");
        let change = if removal {
            delete_sql(ps[0])
        } else {
            insert_version_sql(p)
        };
        let changer = spawn_session(t, &[s("BEGIN"), change, s("COMMIT")]);
        assert!(
            wait_until_blocked(&pool, &[t_pid]).await,
            "{label}: the change must wait for the deletion"
        );
        run(&mut t2, "COMMIT")
            .await
            .unwrap_or_else(|error| panic!("{label}, deletion: {error}"));
        let outcome = outcome_of(changer).await;
        if removal {
            outcome.unwrap_or_else(|error| panic!("{label}: {error}"));
        } else {
            let error = outcome.expect_err("a version of a deleted profile");
            assert_eq!(
                sqlstate(&error).as_deref(),
                Some(FOREIGN_KEY_VIOLATION),
                "{label}: wrong mechanism: {error}"
            );
        }
        assert_eq!(version_count(&pool, p).await, 0, "{label}");
        assert_eq!(guard_rows(&pool, p).await, 0, "{label}");
    }
}
