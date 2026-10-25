-- The guard row of a profile goes with its profile (#75).
--
-- Migration 20260919090000 gave `profile_settings_guard` no foreign key to
-- `profile`, and its guard rows outlived their profiles. Deleting a profile
-- (`DELETE /profiles/{id}`, #75) would have left one behind each time. This
-- migration ties the guard row to its profile with `ON DELETE CASCADE`, so no
-- deletion of a profile, whatever its path, can forget it.
--
-- That migration explains why it chose no foreign key: writing a guard row
-- would take a `FOR KEY SHARE` lock on the profile row, and bring whoever
-- locks that row back into the waits of the check. With a foreign key, that
-- lock is taken only when a guard row is **inserted**, or updated again by the
-- transaction that last wrote it: PostgreSQL checks the foreign key of an
-- `UPDATE` that leaves `profile_id` unchanged only when the row version being
-- updated is the transaction's own. The check writes the guard row twice at
-- COMMIT — once in its ordered pass, once for the profile it checks — so the
-- second write would take that lock. `write_profile_settings_guard` therefore
-- no longer updates a guard row this transaction already wrote: `ON CONFLICT
-- DO UPDATE ... WHERE` still locks the row when its condition is false, and
-- the row lock is all the serialisation needs. A guard row written inside a
-- subtransaction carries the subtransaction's id, not the transaction's, and
-- is updated again: that happens only to a check run early, by `SET
-- CONSTRAINTS ... IMMEDIATE` inside a savepoint, which can already deadlock.
-- And every profile gets its guard row
-- when it is created, by a trigger on `profile`, in the transaction that
-- creates it — a profile being created is invisible to every other
-- transaction, so nothing can wait on it there. Every later check finds the
-- row and updates it, taking no lock on the profile, and the six interleavings
-- of plain DML that migration measured (a profile locked `FOR UPDATE` or
-- being deleted while one of its versions is removed) still commit. The
-- profiles that exist when this migration runs get their guard row here.
--
-- What the foreign key brings instead:
--
--   * a deletion of a profile now also deletes its guard row, so the check is
--     no longer the only writer of guard rows. It still does not wait for a
--     deletion, because the deletion reaches the guard row after the
--     versions: `profile` fires its cascades in the order of their trigger
--     names, `RI_ConstraintTrigger_a_<oid>`, and the constraint added below
--     is the latest of the foreign keys towards `profile`, so its triggers
--     have the largest oids. A transaction whose check is about to write the
--     guard row of a profile has removed one of its versions, and holds that
--     version row: a deletion of the profile waits for it in its cascade
--     towards the versions, before it holds the guard row, and the check can
--     write it at COMMIT. Were the guard cascade to run first, the two would
--     deadlock. The order rests on the names comparing as the oids do, which
--     holds as long as both oids have the same number of digits, and on the
--     foreign keys keeping their creation order — which a restore does not:
--     `pg_dump` recreates them in alphabetical order, the guard's before the
--     versions', and the guard cascade then runs first. `DELETE
--     /profiles/{id}` therefore does not rely on that order: it locks the
--     profile row, then its versions, before deleting the profile, and waits
--     for a removal holding nothing its COMMIT needs. A plain `DELETE FROM
--     profile` that does not take those locks first, run on a database
--     whose guard cascade runs first, can deadlock (40P01) with a concurrent
--     removal of one of its versions: PostgreSQL aborts one of the two, which
--     leaves nothing half done and can be replayed;
--   * a guard row deleted by hand, class (b) of migration 20260919090000, is
--     written again by an `INSERT`, which takes the `FOR KEY SHARE` lock: that
--     check can then wait for, and deadlock with, a transaction holding the
--     profile row `FOR UPDATE`;
--   * the ordered pass of the check, at COMMIT, may meet a profile its own
--     transaction deleted after removing one of its versions. The guard row
--     went with the profile, and writing it again would be refused by the
--     foreign key: `write_profile_settings_guard` now writes nothing for a
--     profile that no longer exists. A removal racing the deletion of the
--     same profile by another transaction does not get there: either it
--     holds the version the deletion has to cascade into, and the deletion
--     waits for it, or it waits for the deletion and then finds nothing left
--     to remove.
--
-- A role that creates profiles now needs INSERT on `profile_settings_guard`,
-- since the trigger function runs with the privileges of the caller; without
-- it, the creation fails with 42501.
--
-- Every relation and type is named schema-qualified: class (e) of the initial
-- migration, `pg_temp` shadowing.

DELETE FROM public.profile_settings_guard AS g
WHERE NOT EXISTS (SELECT 1 FROM public.profile AS p WHERE p.id = g.profile_id);

INSERT INTO public.profile_settings_guard (profile_id)
SELECT p.id FROM public.profile AS p
ON CONFLICT (profile_id) DO NOTHING;

ALTER TABLE public.profile_settings_guard
    ADD CONSTRAINT profile_settings_guard_profile_id_fkey
    FOREIGN KEY (profile_id) REFERENCES public.profile (id) ON DELETE CASCADE;

COMMENT ON TABLE public.profile_settings_guard IS
    'One row per profile, written at its creation and deleted with it; written by the settings-version check to serialise concurrent removals (#43, #75). See migrations 20260919090000 and 20261025090000.';

CREATE OR REPLACE FUNCTION public.write_profile_settings_guard(target_profile pg_catalog.uuid)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO public.profile_settings_guard AS g (profile_id)
    SELECT target_profile
    WHERE EXISTS (SELECT 1 FROM public.profile AS p WHERE p.id = target_profile)
    ON CONFLICT (profile_id) DO UPDATE SET writes = g.writes + 1
    WHERE g.xmin OPERATOR(pg_catalog.<>) pg_catalog.pg_current_xact_id()::pg_catalog.xid;
$$;

COMMENT ON FUNCTION public.write_profile_settings_guard(pg_catalog.uuid) IS
    'Writes the guard row of a profile, creating it if needed; only locks it when this transaction already wrote it; writes nothing for a profile that no longer exists (#43, #75).';

CREATE FUNCTION public.create_profile_settings_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.profile_settings_guard (profile_id)
    VALUES (NEW.id)
    ON CONFLICT (profile_id) DO NOTHING;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION public.create_profile_settings_guard() IS
    'Writes the guard row of a profile in the transaction that creates it, so that the settings-version check only ever updates it (#75).';

CREATE TRIGGER profile_creates_settings_guard
    AFTER INSERT ON public.profile
    FOR EACH ROW EXECUTE FUNCTION public.create_profile_settings_guard();
