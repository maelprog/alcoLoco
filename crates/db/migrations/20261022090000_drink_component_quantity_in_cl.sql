-- A component quantity of a drink in cL is stored in centilitres (#14).
--
-- The initial migration declared it in millilitres, but nothing ever wrote it
-- so: the drink API of #14 is the first writer. The API exchanges cL
-- quantities in centilitres (user decision on #14), and storing them in the
-- unit they travel in is the only way a quantity reads back exactly as it was
-- written: converting with `x * 10` and back with `/ 10` does not return `x`
-- for every float8 — 839.48 does not — and `x * 10` maps distinct values to
-- the same one, so no inverse can repair it.
--
-- Only the meaning of the cL case changes. In percent the quantity is still a
-- share of `drink.volume_total_ml`, which stays in millilitres, and the check
-- of migration 20261021090000 (at most 100 % in percent, one or more
-- components) reads no cL quantity. The conversion to millilitres, the unit of
-- the computation (SPEC.md §6.1), is the reader's: `crates/api` applies it
-- before the rules of `crates/domain`.
--
-- The initial migration is append-only, hence this file.

COMMENT ON COLUMN public.drink_component.quantity IS
    'Centilitres when the parent drink is in cl (as entered; multiply by 10 for millilitres), percent of drink.volume_total_ml when it is in percent.';
