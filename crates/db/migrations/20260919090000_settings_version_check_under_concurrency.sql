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
-- What changes: the check no longer only *reads* the versions of the profile,
-- it first *writes* the profile row. Two transactions removing versions of the
-- same profile therefore write the same row, and PostgreSQL serialises writers
-- of one row whatever their isolation level:
--
--   * READ COMMITTED: the second writer waits until the first has committed,
--     then re-reads with a fresh snapshot that does include that commit — row
--     locks are released only after the committing transaction has left
--     `ProcArray`. The `EXISTS` that follows the write sees the first
--     transaction's removals, and a profile left empty is refused by the check
--     itself (SQLSTATE 23000, `profile ... has no settings version`);
--   * REPEATABLE READ and SERIALIZABLE: the second writer cannot re-read, so it
--     gets `could not serialize access due to concurrent update` (40001) as
--     soon as it touches a row the first one has updated since its snapshot.
--     That is the only answer a transaction reading through an older snapshot
--     can honestly give, and it is a retryable one. Its price: at those two
--     levels, **any** two transactions removing versions of the same profile
--     concurrently fail one of them, including when versions remain.
--
-- A plain row lock would not do. `SELECT ... FOR UPDATE` on the profile row,
-- tried during the arbitration of #2, closes READ COMMITTED only: a lock
-- leaves no new row version behind, so a REPEATABLE READ transaction obtains
-- it once the other has committed and then reads its old snapshot anyway. The
-- write is what leaves a trace the second transaction trips over.
--
-- Deadlocks. That same `FOR UPDATE` attempt also introduced deadlocks: the
-- deferred triggers fire at COMMIT in the order their row events were queued,
-- so two transactions removing versions of several profiles in opposite
-- orders locked the profile rows in opposite orders. Here the first firing of
-- the check in a transaction locks **every** profile the transaction has
-- removed a version from, in one statement, in `id` order; every later firing
-- finds its row already locked. The set comes from an immediate row trigger
-- that records the profile ids in a transaction-local setting,
-- `alcoloco.settings_version_profiles`. The setting only decides the *order*
-- in which locks are taken, never *whether* a profile is checked: each firing
-- still writes and checks its own profile. A session that tampers with the
-- setting can therefore lose the ordering, and deadlock itself, but cannot
-- skip the check. Savepoints need no care: a rolled back subtransaction takes
-- both its row events and its change to the setting with it.
--
-- The same immediate trigger takes a `FOR KEY SHARE` lock on the profile row
-- when the version is removed, which is the lock a foreign key check takes
-- when a version is inserted. It makes a concurrent `DELETE FROM profile` wait
-- for the transaction instead of cascading into the version it has already
-- removed: without it, that DELETE would hold the profile row while waiting
-- for the version, and the check would wait for the profile row at COMMIT.
-- `FOR KEY SHARE` does not conflict with the commit-time write, which updates
-- no key column, nor with another transaction's `FOR KEY SHARE`.
--
-- The write sets `profile.updated_at`. `set_updated_at()` would set it anyway,
-- on any UPDATE of the row; it is spelled out here so that the side effect is
-- read where it happens: removing or moving away a settings version of a
-- profile now counts as updating that profile. Inserting one does not, since
-- an insertion cannot empty a profile and takes no part in the check.
--
-- What is still not covered, stated as in the initial migration: classes (a)
-- `TRUNCATE`, (b) switching the triggers off, and (c) the writing transaction
-- reading its own intermediate state are unchanged. So is one more deadlock
-- this check can take part in: a transaction that has itself updated a
-- profile row (say, renamed profile X) and then removes a version of profile
-- Y holds X before the commit-time pass; a second transaction doing the
-- converse holds Y. At COMMIT each pass waits for the row the other holds.
-- The ordered pass removes the deadlocks the check would create on its own,
-- not those between its locks and the ones the application took earlier.
--
-- Both relations are read schema-qualified, as every function of this schema
-- does: class (e) of the initial migration, `pg_temp` shadowing.

CREATE FUNCTION note_profile_settings_version_removal() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    pending text;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.profile_id = OLD.profile_id THEN
        RETURN NULL;
    END IF;

    -- A profile already deleted by this transaction (its versions are being
    -- cascaded away) has nothing left to guarantee, and FOUND is false.
    PERFORM 1 FROM public.profile WHERE id = OLD.profile_id FOR KEY SHARE;
    IF NOT FOUND THEN
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
    'Records, in a transaction-local setting, each profile a version is removed from, so that the commit-time check locks them all in id order (#43).';

CREATE FUNCTION assert_profile_keeps_a_settings_version() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    pending text := current_setting('alcoloco.settings_version_profiles', true);
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.profile_id = OLD.profile_id THEN
        RETURN NULL;
    END IF;

    -- First firing in this transaction: lock every recorded profile, in one
    -- global order, before writing any of them.
    IF pending <> '' THEN
        PERFORM set_config('alcoloco.settings_version_profiles', '', true);
        PERFORM 1
        FROM public.profile
        WHERE id = ANY (string_to_array(pending, ',')::uuid[])
        ORDER BY id
        FOR NO KEY UPDATE;
    END IF;

    -- The write that serialises the writers of this profile. No row: the
    -- profile itself is gone (its versions were cascaded away), nothing to
    -- guarantee any more.
    UPDATE public.profile SET updated_at = now() WHERE id = OLD.profile_id;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    -- A new statement, hence under READ COMMITTED a new snapshot, taken after
    -- any concurrent writer of this profile row has committed.
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
    'Refuses a COMMIT that leaves a profile without a settings version, concurrent writers included: writes the profile row before checking (#43).';

DROP TRIGGER profile_settings_version_keeps_last ON profile_settings_version;

CREATE TRIGGER profile_settings_version_notes_removal
    AFTER DELETE OR UPDATE OF profile_id ON profile_settings_version
    FOR EACH ROW EXECUTE FUNCTION note_profile_settings_version_removal();

CREATE CONSTRAINT TRIGGER profile_settings_version_keeps_last
    AFTER DELETE OR UPDATE OF profile_id ON profile_settings_version
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_profile_keeps_a_settings_version();
