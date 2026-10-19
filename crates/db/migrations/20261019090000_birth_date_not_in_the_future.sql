-- A settings version may not carry a birth date in the future (#7).
--
-- SPEC.md §10.0-K makes the birth date a direct input of the computation: the
-- age Watson uses is derived from it at the ingestion time of each drink. The
-- initial schema put no bound on it, and a version born in 2099 was accepted
-- (measured during the verification of #2). The API already refuses such a
-- date, as a 400 naming the field; this file makes the database refuse it too,
-- for every writer that does not go through the API.
--
-- The initial migration is append-only, hence this file rather than a CHECK
-- added there.
--
-- The rule is the API's: the last date accepted is **today in UTC**, the
-- calendar every date crossing the API is expressed in. "Today" is read from
-- `now()`, the start of the writing transaction.
--
-- A trigger rather than a CHECK. PostgreSQL does accept `CHECK (birth_date <=
-- now()::date)`, but it treats every CHECK as immutable — re-checked as-is by
-- a restore, assumed by the planner — and this one is not. The bound only ever
-- loosens as time passes, so a row accepted once stays acceptable; a trigger
-- says outright that the rule is evaluated at write time, and checks nothing
-- else.
--
-- Rows already stored are not re-examined: a trigger fires on writes only. The
-- one way such a row exists is a write made before this migration, through a
-- path other than the API.
--
-- `valid_from` in the future stays refused by the API only (SPEC.md §10.0-J),
-- as the initial schema chose: the default `valid_from` is the clock of the
-- API, and checking it against the clock of the database would refuse every
-- default write the day the two drift by a millisecond in that direction.
--
-- The function reads no relation and names its one type-dependent expression
-- with `pg_catalog`, as every function of this schema does: class (e) of the
-- initial migration, `pg_temp` shadowing.

CREATE FUNCTION assert_birth_date_not_in_the_future() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.birth_date > (pg_catalog.now() AT TIME ZONE 'UTC')::pg_catalog.date THEN
        RAISE EXCEPTION 'birth date % is in the future', NEW.birth_date
            USING ERRCODE = 'check_violation';
    END IF;

    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION assert_birth_date_not_in_the_future() IS
    'Refuses a settings version born after today (UTC); the age of SPEC.md §10.0-K is derived from it (#7).';

CREATE TRIGGER profile_settings_version_birth_date_not_in_the_future
    BEFORE INSERT OR UPDATE OF birth_date ON profile_settings_version
    FOR EACH ROW EXECUTE FUNCTION assert_birth_date_not_in_the_future();
