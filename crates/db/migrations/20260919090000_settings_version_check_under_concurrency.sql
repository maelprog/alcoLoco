-- Every profile keeps a settings version under concurrent writes too (#43).
--
-- The initial schema checks "a profile has at least one settings version" with
-- a deferred constraint trigger that evaluates an `EXISTS` at COMMIT. That
-- check is sound for a transaction running alone and unsound under
-- concurrency — class (d) of the initial migration: two transactions each
-- removing a *different* version of the same profile both commit, each having
-- seen the version the other was removing, and the profile is left durably at
-- zero versions. Under REPEATABLE READ the check reads through a snapshot taken
-- before the other transaction committed, so no distance between the two
-- commits helps; under READ COMMITTED the snapshot is fresh, but is taken
-- before the other transaction has finished committing.
--
-- The initial migration is append-only, so its comments still describe class
-- (d) as open and still tell the readers of a profile's settings not to rely on
-- the invariant under concurrent writes "for as long as #43 is open". Both
-- statements stop being true with this file, which closes class (d); the rest
-- of that description is the history of the defect.
--
-- This migration replaces the check on the `profile_settings_version` side
-- (DELETE and UPDATE OF profile_id). The `profile` side (a profile inserted
-- without a version) is left as it is: a profile being inserted is invisible
-- to every other transaction until it commits, so nothing can race it.
--
-- The check no longer only *reads* the versions of a profile: it first
-- *writes* a row of its own for that profile, in `profile_settings_guard`, and
-- reads the versions afterwards. Two transactions removing versions of the same
-- profile therefore write the same guard row, and PostgreSQL serialises writers
-- of one row whatever their isolation level:
--
--   * READ COMMITTED: the second writer waits until the first has committed,
--     then re-reads with a fresh snapshot that does include that commit — a
--     transaction releases its row locks only after it has left `ProcArray`.
--     The `EXISTS` that follows the write sees the first transaction's
--     removals, and a profile left empty is refused by the check itself
--     (SQLSTATE 23000, `profile ... has no settings version`);
--   * REPEATABLE READ and SERIALIZABLE: the second writer cannot re-read, so it
--     gets `could not serialize access due to concurrent update` (40001) as
--     soon as it touches a guard row the first one has written since its
--     snapshot. That is the only answer a transaction reading through an older
--     snapshot can honestly give, and it is a retryable one. Its price: at
--     those two levels, **any** two transactions removing versions of the same
--     profile concurrently fail one of them, including when versions remain.
--
-- Why a write and not a lock: `SELECT ... FOR UPDATE`, tried during the
-- arbitration of #2, closes READ COMMITTED only. A lock leaves no new row
-- version behind, so a REPEATABLE READ transaction obtains it once the other
-- has committed and then reads its old snapshot anyway.
--
-- Why a table of its own and not the `profile` row. Writing the profile row,
-- the first version of this fix, made the check wait at COMMIT for whatever
-- lock the application or a cascade held on that row — an UPDATE of the
-- profile, a `SELECT ... FOR SHARE` or `FOR UPDATE`, a `DELETE FROM profile`
-- waiting to cascade into the very version being removed — while the holder
-- could be waiting for the checking transaction itself: six scenarios of plain
-- DML that commit on the initial schema deadlocked there. Nothing but this
-- check ever locks a guard row, so the check waits only for other checks. For
-- the same reason the guard table has **no foreign key** to `profile`: writing
-- a guard row would otherwise take a `FOR KEY SHARE` lock on the profile row
-- and bring the profile's lockers back into the check's waits. A guard row
-- outlives its profile; it is a few bytes, carries no data, and is recreated
-- on demand if deleted, since the check writes it with `INSERT ... ON
-- CONFLICT DO UPDATE`. Deleting or emptying the table weakens nothing.
--
-- Deadlocks between checks. The deferred triggers fire at COMMIT in the order
-- their row events were queued, so two transactions removing versions of
-- several profiles in opposite orders would write the guard rows in opposite
-- orders. The first firing of the check in a transaction therefore writes
-- **every** guard row the transaction needs, in `profile_id` order; every
-- later firing finds its row already written by its own transaction. The set
-- comes from an immediate row trigger that records the profile ids in a
-- transaction-local setting, `alcoloco.settings_version_profiles`. The setting
-- only decides the *order* in which guard rows are written, never *whether* a
-- profile is checked: each firing still writes its own guard row and checks
-- its own profile. A session that tampers with the setting can therefore lose
-- the ordering, and deadlock against another check, but cannot skip the check.
-- Savepoints need no care: a rolled back subtransaction takes both its row
-- events and its change to the setting with it.
--
-- The setting is a comma-separated text that the immediate trigger searches
-- and rewrites at every row, so a transaction removing versions of N distinct
-- profiles costs O(N²): a single DELETE of one version from each of 16 000
-- profiles took about 12 s instead of 7 ms on the initial schema, and 47 s at
-- 32 000 (measured on PostgreSQL 16.15 while closing #43). The cost is per
-- transaction, since the setting lives as long as the transaction does.
--
-- What remains, and where the "no new deadlock" argument stops: a guard row is
-- held from the moment the check writes it to the end of the transaction. At
-- COMMIT that is the end of the transaction's work, so the holder waits for
-- nothing but other guard rows, taken in one order. `SET CONSTRAINTS ...
-- IMMEDIATE` runs the check in the middle of a transaction instead; the
-- transaction then keeps its guard rows while it goes on with other
-- statements, and can deadlock against another check. So can a role that
-- writes `profile_settings_guard` itself. Classes (a) `TRUNCATE`, (b) switching
-- the triggers off, and (c) the writing transaction reading its own
-- intermediate state are unchanged.
--
-- A role that removes settings versions needs INSERT, UPDATE and SELECT on
-- `profile_settings_guard`: the trigger functions run with the privileges of
-- the caller, and the `ON CONFLICT DO UPDATE` of the check reads the existing
-- guard row. Without any of the three, the COMMIT fails with 42501.
--
-- Every relation is read schema-qualified, as every function of this schema
-- does: class (e) of the initial migration, `pg_temp` shadowing.

CREATE TABLE profile_settings_guard (
    profile_id uuid PRIMARY KEY,
    writes bigint NOT NULL DEFAULT 1
);

COMMENT ON TABLE profile_settings_guard IS
    'One row per profile whose settings versions were ever removed; written by the settings-version check to serialise concurrent removals (#43). No foreign key on purpose: see migration 20260919090000.';

CREATE FUNCTION note_profile_settings_version_removal() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    pending text;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.profile_id = OLD.profile_id THEN
        RETURN NULL;
    END IF;

    -- A profile already deleted by this transaction (its versions are being
    -- cascaded away) has nothing left to guarantee. A plain read: it locks
    -- nothing, so it cannot make this statement wait.
    IF NOT EXISTS (SELECT 1 FROM public.profile WHERE id = OLD.profile_id) THEN
        RETURN NULL;
    END IF;

    pending := coalesce(current_setting('alcoloco.settings_version_profiles', true), '');
    IF position(OLD.profile_id::text IN pending) = 0 THEN
        PERFORM set_config(
            'alcoloco.settings_version_profiles',
            concat_ws(',', nullif(pending, ''), OLD.profile_id::text),
            true
        );
    END IF;

    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION note_profile_settings_version_removal() IS
    'Records, in a transaction-local setting, each profile a version is removed from, so that the commit-time check writes their guard rows in profile_id order (#43).';

CREATE FUNCTION write_profile_settings_guard(target_profile uuid) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO public.profile_settings_guard AS g (profile_id)
    VALUES (target_profile)
    ON CONFLICT (profile_id) DO UPDATE SET writes = g.writes + 1;
$$;

COMMENT ON FUNCTION write_profile_settings_guard(uuid) IS
    'Writes the guard row of a profile, creating it if needed (#43).';

CREATE FUNCTION assert_profile_keeps_a_settings_version() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    pending text := current_setting('alcoloco.settings_version_profiles', true);
    each_profile uuid;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.profile_id = OLD.profile_id THEN
        RETURN NULL;
    END IF;

    -- First firing in this transaction: write every recorded guard row, in one
    -- global order, before anything else can make this transaction wait.
    IF pending <> '' THEN
        PERFORM set_config('alcoloco.settings_version_profiles', '', true);
        FOR each_profile IN
            SELECT DISTINCT p FROM unnest(string_to_array(pending, ',')::uuid[]) AS p ORDER BY p
        LOOP
            PERFORM public.write_profile_settings_guard(each_profile);
        END LOOP;
    END IF;

    -- The profile itself is gone (its versions were cascaded away): nothing to
    -- guarantee any more. A plain read, which locks nothing.
    IF NOT EXISTS (SELECT 1 FROM public.profile WHERE id = OLD.profile_id) THEN
        RETURN NULL;
    END IF;

    -- The write that serialises the checks of this profile. Already done by
    -- the ordered pass above unless the setting was tampered with; doing it
    -- again costs a row version and keeps the check independent of the setting.
    PERFORM public.write_profile_settings_guard(OLD.profile_id);

    -- A new statement, hence under READ COMMITTED a new snapshot, taken after
    -- any concurrent writer of this guard row has committed.
    IF NOT EXISTS (
        SELECT 1 FROM public.profile_settings_version WHERE profile_id = OLD.profile_id
    ) THEN
        RAISE EXCEPTION 'profile % has no settings version', OLD.profile_id
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION assert_profile_keeps_a_settings_version() IS
    'Refuses a COMMIT that leaves a profile without a settings version, concurrent writers included: writes the profile guard row before checking (#43).';

DROP TRIGGER profile_settings_version_keeps_last ON profile_settings_version;

CREATE TRIGGER profile_settings_version_notes_removal
    AFTER DELETE OR UPDATE OF profile_id ON profile_settings_version
    FOR EACH ROW EXECUTE FUNCTION note_profile_settings_version_removal();

CREATE CONSTRAINT TRIGGER profile_settings_version_keeps_last
    AFTER DELETE OR UPDATE OF profile_id ON profile_settings_version
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_profile_keeps_a_settings_version();
