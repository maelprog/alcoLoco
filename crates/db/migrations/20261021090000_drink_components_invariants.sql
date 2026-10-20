-- The structural rules of a drink's components (#13, SPEC.md §4, §5.3,
-- §10.0-A), enforced by the database for every writer.
--
-- The initial schema left three of them open, all reproduced on it:
--
--   1. SPEC.md §4 gives a drink "one or more" components. Nothing refused a
--      drink committed without any, or a COMMIT removing the last one.
--   2. In percent, the components of a drink are shares of its total volume and
--      sum to at most 100 %. `drink_component.quantity` was only bounded below:
--      200 % + 100 % of a 500 mL drink was accepted, and SPEC.md §6.1 would
--      then compute more alcohol than the drink holds.
--   3. `quantity > 0` and `volume_total_ml > 0` let NaN and Infinity through,
--      since PostgreSQL orders NaN above every number.
--
-- The initial migration is append-only, hence this file.
--
-- (3) is a plain CHECK on each row: `< 'Infinity'` refuses both values, NaN
-- included. It validates the rows already stored when it is added: a database
-- holding such a quantity or volume fails this migration until the row is fixed.
--
-- (1) and (2) are properties of a drink and of all its components together,
-- which no CHECK can express. They are checked by one deferred constraint
-- trigger per table, which evaluates the drink at COMMIT, whatever the order
-- in which the transaction wrote it: a drink is inserted before its
-- components, and replacing every component of a drink goes through an empty
-- drink. Rows already stored are not re-examined: a trigger fires on writes
-- only, and the one way such a row exists is a write made before this
-- migration, through a path other than the API.
--
-- What makes the check of a drink run:
--
--   * `drink`: INSERT, and UPDATE OF `id` (a drink renumbered after its
--     components were deleted) or `quantity_unit` (a drink switched to percent
--     reads its component quantities as shares from then on);
--   * `drink_component`: INSERT, DELETE, UPDATE OF `drink_id` or `quantity`,
--     for the drink the row leaves and the drink it joins.
--
-- What it refuses, with SQLSTATE 23000 (`integrity_constraint_violation`):
-- `drink ... has no component`, and, for a drink in percent, `components of
-- drink ... exceed 100 %`. The bound tolerates 1e-9 percentage point, the
-- `PERCENT_SUM_TOLERANCE` of `crates/domain/src/drink.rs`: shares such as
-- 0.2 + 83.9 + 15.9 sum to 100.00000000000001 in float8, and the database must
-- accept every drink the domain accepts. A drink deleted by the transaction is
-- not checked: its components are gone with it.
--
-- Under concurrency the check follows #43 (migration 20260919090000), and its
-- comment holds here word for word with "drink" for "profile". In short: a
-- check that only reads is unsound when two transactions each remove a
-- different one of the last two components of a drink, or each add a share
-- that fits alone, or one switches the drink to percent while the other adds
-- a component — each commits having seen the drink without the other's write.
-- So the check first *writes* the drink's row in `drink_component_guard`, then
-- reads. Writers of one row are serialised whatever their isolation level:
-- under READ COMMITTED the later one waits for the earlier one's COMMIT and
-- then reads it, and is refused by the check itself (23000); under REPEATABLE
-- READ and SERIALIZABLE it gets `could not serialize access due to concurrent
-- update` (40001), retryable. At those two levels any two transactions writing
-- the components of the same drink concurrently fail one of them.
--
-- The guard is a table of its own, without foreign key to `drink`, so that the
-- check never waits for a lock the application or a cascade holds on the drink
-- row. A guard row outlives its drink. Unlike the settings guard, which only
-- removals write, every drink written gets one guard row: a few bytes, no
-- data. Deleting guard rows reopens the concurrency hole under REPEATABLE
-- READ, exactly as for #43; the application never does.
--
-- The first firing of the check in a transaction writes the guard rows of
-- every drink the transaction touched, in `id` order, so that two transactions
-- writing several drinks in opposite orders do not deadlock at COMMIT. The set
-- comes from an immediate row trigger recording the drink ids in a
-- transaction-local setting, `alcoloco.drink_component_drinks`; it decides the
-- order only, never whether a drink is checked. Same O(N²) cost in the number
-- of distinct drinks per transaction as #43, and the same limits: `SET
-- CONSTRAINTS ... IMMEDIATE`, or a transaction that both removes settings
-- versions and writes drinks while another does the same on the same profile
-- and drink, takes guard rows of the two families in event order and can
-- deadlock. Classes (a) `TRUNCATE`, (b) switching triggers off and (c) a
-- transaction reading its own intermediate state of the initial migration
-- apply here unchanged.
--
-- A role writing drinks or components needs INSERT, UPDATE and SELECT on
-- `drink_component_guard`; without them the COMMIT fails with 42501.
--
-- Every relation and type is named schema-qualified: class (e) of the initial
-- migration, `pg_temp` shadowing.

ALTER TABLE drink_component
    ADD CONSTRAINT drink_component_quantity_finite CHECK (quantity < 'Infinity');

ALTER TABLE drink
    ADD CONSTRAINT drink_volume_total_finite CHECK (volume_total_ml < 'Infinity');

CREATE TABLE drink_component_guard (
    drink_id uuid PRIMARY KEY,
    writes bigint NOT NULL DEFAULT 1
);

COMMENT ON TABLE drink_component_guard IS
    'One row per drink ever written; written by the drink check to serialise concurrent writes of its components (#13). No foreign key on purpose: see migration 20261021090000.';

CREATE FUNCTION note_drink_to_check(target_drink pg_catalog.uuid) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    pending pg_catalog.text :=
        coalesce(current_setting('alcoloco.drink_component_drinks', true), '');
BEGIN
    -- A drink already deleted by this transaction has nothing left to check.
    -- A plain read: it locks nothing.
    IF NOT EXISTS (SELECT 1 FROM public.drink WHERE id = target_drink) THEN
        RETURN;
    END IF;

    IF position(target_drink::pg_catalog.text IN pending) = 0 THEN
        PERFORM set_config(
            'alcoloco.drink_component_drinks',
            concat_ws(',', nullif(pending, ''), target_drink::pg_catalog.text),
            true
        );
    END IF;
END;
$$;

COMMENT ON FUNCTION note_drink_to_check(pg_catalog.uuid) IS
    'Records, in a transaction-local setting, a drink written by the transaction, so that the commit-time check writes the guard rows in id order (#13).';

CREATE FUNCTION write_drink_component_guard(target_drink pg_catalog.uuid) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO public.drink_component_guard AS g (drink_id)
    VALUES (target_drink)
    ON CONFLICT (drink_id) DO UPDATE SET writes = g.writes + 1;
$$;

COMMENT ON FUNCTION write_drink_component_guard(pg_catalog.uuid) IS
    'Writes the guard row of a drink, creating it if needed (#13).';

CREATE FUNCTION assert_drink_components_hold(target_drink pg_catalog.uuid) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    pending pg_catalog.text := current_setting('alcoloco.drink_component_drinks', true);
    each_drink pg_catalog.uuid;
    unit public.quantity_unit;
    component_count pg_catalog.int8;
    percent_sum pg_catalog.float8;
BEGIN
    -- First firing in this transaction: write every recorded guard row, in one
    -- global order, before anything else can make this transaction wait.
    IF pending <> '' THEN
        PERFORM set_config('alcoloco.drink_component_drinks', '', true);
        FOR each_drink IN
            SELECT DISTINCT d
            FROM unnest(string_to_array(pending, ',')::pg_catalog.uuid[]) AS d
            ORDER BY d
        LOOP
            PERFORM public.write_drink_component_guard(each_drink);
        END LOOP;
    END IF;

    -- Deleted by this transaction: nothing to check. A plain read.
    IF NOT EXISTS (SELECT 1 FROM public.drink WHERE id = target_drink) THEN
        RETURN;
    END IF;

    -- The write that serialises the checks of this drink; already done by the
    -- ordered pass unless the setting was tampered with.
    PERFORM public.write_drink_component_guard(target_drink);

    -- New statements, hence under READ COMMITTED a new snapshot, taken after
    -- any concurrent writer of this guard row has committed.
    SELECT d.quantity_unit INTO unit FROM public.drink AS d WHERE d.id = target_drink;
    IF NOT FOUND THEN
        RETURN;
    END IF;

    SELECT count(*), coalesce(sum(c.quantity), 0)
    INTO component_count, percent_sum
    FROM public.drink_component AS c
    WHERE c.drink_id = target_drink;

    IF component_count = 0 THEN
        RAISE EXCEPTION 'drink % has no component', target_drink
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    -- 1e-9: PERCENT_SUM_TOLERANCE of crates/domain/src/drink.rs.
    IF unit = 'percent'::public.quantity_unit AND percent_sum > 100 + 1e-9 THEN
        RAISE EXCEPTION 'components of drink % exceed 100 %%', target_drink
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END;
$$;

COMMENT ON FUNCTION assert_drink_components_hold(pg_catalog.uuid) IS
    'Refuses a COMMIT leaving a drink without component, or in percent above 100 %, concurrent writers included: writes the drink guard row before checking (#13).';

-- Row trigger functions: which drinks a row event concerns.

CREATE FUNCTION note_drink_component_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') THEN
        PERFORM public.note_drink_to_check(OLD.drink_id);
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND (TG_OP = 'INSERT' OR NEW.drink_id <> OLD.drink_id) THEN
        PERFORM public.note_drink_to_check(NEW.drink_id);
    END IF;
    RETURN NULL;
END;
$$;

CREATE FUNCTION check_drink_component_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') THEN
        PERFORM public.assert_drink_components_hold(OLD.drink_id);
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND (TG_OP = 'INSERT' OR NEW.drink_id <> OLD.drink_id) THEN
        PERFORM public.assert_drink_components_hold(NEW.drink_id);
    END IF;
    RETURN NULL;
END;
$$;

CREATE FUNCTION note_drink_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM public.note_drink_to_check(NEW.id);
    RETURN NULL;
END;
$$;

CREATE FUNCTION check_drink_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM public.assert_drink_components_hold(NEW.id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER drink_component_notes_write
    AFTER INSERT OR DELETE OR UPDATE OF drink_id, quantity ON drink_component
    FOR EACH ROW EXECUTE FUNCTION note_drink_component_write();

CREATE CONSTRAINT TRIGGER drink_component_keeps_drink_valid
    AFTER INSERT OR DELETE OR UPDATE OF drink_id, quantity ON drink_component
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_drink_component_write();

CREATE TRIGGER drink_notes_write
    AFTER INSERT OR UPDATE OF id, quantity_unit ON drink
    FOR EACH ROW EXECUTE FUNCTION note_drink_write();

CREATE CONSTRAINT TRIGGER drink_keeps_its_components_valid
    AFTER INSERT OR UPDATE OF id, quantity_unit ON drink
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_drink_write();
