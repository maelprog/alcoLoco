# Migrations

Every `<version>_<description>.sql` file of this directory is embedded in the
`db` crate at compile time by `sqlx::migrate!("./migrations")`
(`crates/db/src/lib.rs`) and applied in version order. sqlx skips every file
whose name is not `<something>_<something>.sql`, so this README is not a
migration. Any other file kept here must not be named that way: sqlx would
take it for a migration, and a prefix that is not an integer fails the build.

## Append-only

A migration merged to `main` is never edited again, not even in a comment.

sqlx records each applied migration in `_sqlx_migrations` together with a
checksum of its **whole file**, comments and whitespace included. On every
`migrate`, it recomputes the checksum of each file already applied and refuses
to go on when one differs (`migration … was previously applied but has been
modified`). Editing a merged file therefore breaks `migrate` on every database
already migrated — development, CI fixtures, production alike.

A schema change goes in a new file. A comment that has become wrong goes in
the errata below.

## Errata

Comments of merged migrations that are known to be wrong. The file stays as it
is; what is true is stated here, with the place that carries the correct
statement. Line numbers are those of the file as merged — it can no longer
change.

### 1. `20260819090000_initial_schema.sql:113-121` — future `valid_from`

What the file says:

- a CHECK cannot refuse a future `valid_from` "since it may not call `now()`";
- the rule, and the replacement of later versions, "belong to the write policy
  of issue #7", and "until #7 lands, a future `valid_from` is accepted by the
  database".

What is true:

- PostgreSQL 16 does accept a CHECK calling `now()`. It is not used because
  PostgreSQL treats every CHECK as immutable — re-checked as-is by a restore,
  assumed by the planner — and such a CHECK is not. A rule evaluated at write
  time is a trigger.
- #7 has landed. A future `valid_from` is refused **by the API only**
  (`settings.valid_from`, "must not be in the future"), and the database still
  accepts one, on purpose: the default `valid_from` is the clock of the API,
  and checking it against the clock of the database would refuse every default
  write the day the two drift apart in that direction.
- Posting a version at `T` replaces every version strictly later than `T`;
  the API does it, in the same transaction as the post.

Correct statement: `20261019090000_birth_date_not_in_the_future.sql:17-22`
(CHECK versus trigger) and `:28-31` (`valid_from` refused by the API only).

### 2. `20260819090000_initial_schema.sql:481-482` — curve window of an event

What the file says, in the comment stored on the `event.ends_at` column:
"the curve window is then `min(now(), ends_at)` + 3 h".

What is true (SPEC.md §10.3): the curve of an event runs from its start to
three hours after its end, **even when a fixed end is still to come**; for an
open-ended event (`ends_at` NULL) the upper bound is the instant of the request
plus three hours. Nothing takes the minimum of the two.

Correct statement: no migration carries it. The rule lives in SPEC.md §10.3
and in `EventBacQuery::into_grid` (`crates/api/src/bac/query.rs`). The column
comment itself is stored in the database catalog, so `\d+ event` and
`col_description()` still return the wrong text.

### 3. `20260819090000_initial_schema.sql:215`, `:245-265`, `:291`, `:306-308`, `:318-319`, `:323` and `:326-331` — concurrent removals of settings versions

What the file says: class (d), two concurrent transactions each removing a
different version of the same profile and leaving it without any, is open.
Every place that says so:

- `:215` "Four of the five are open";
- `:245-265`, the description of class (d) itself;
- `:291` "Classes (a), (b) and (d) leave a durably inconsistent state";
- `:306-308` "(c) and (d) are plain DML, so **any** role able to write reaches
  them";
- `:318-319` "two concurrent such `DELETE`s on different versions of one
  profile both commit";
- `:323` "the four open classes";
- `:326-331`: readers of a profile's settings may not rely on the invariant
  under concurrent writes "for as long as #43 is open".

What is true: #43 closed class (d). A profile can no longer be left without a
settings version by concurrent removals; the second writer is refused
(SQLSTATE 23000 under READ COMMITTED, 40001 under REPEATABLE READ and
SERIALIZABLE). Three classes remain open, (a), (b) and (c); of those, only
(a) and (b) leave a durably inconsistent state, and only (c) is reached by plain
DML. The measurements of class (d) remain the history of the defect.

Correct statement:
`20260919090000_settings_version_check_under_concurrency.sql:14-18` (what stops
being true in the initial migration) and `:25-43` (how the check holds).
