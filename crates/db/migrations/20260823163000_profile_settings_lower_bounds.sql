-- Plausible lower bounds on the physiological parameters (issue #6).
--
-- The initial schema bounded weight and height away from zero only:
-- `weight_kg > 0` and `height_cm > 0`. That admits a profile of 0.5 kg and
-- 1 cm, which is not a person, and the API restated those same bounds — so
-- such a profile was accepted with a 201 and fed to the computation engine.
--
-- The bounds added here are stated as "under any living human being", and
-- nothing else: a newborn at term is roughly 2.5 to 4 kg and 48 to 52 cm, the
-- shortest verified adult measured 54.6 cm, and the lightest verified adult
-- weighed about 2.1 kg. Both floors therefore sit below every documented human
-- while rejecting the absurd.
--
-- They are **not** chosen to make any equation behave, and they do not make one
-- behave. Measured against `crates/domain` at the worst corner these bounds
-- accept (2 kg, 40 cm), the male Watson equation of SPEC.md §6.2 still reaches a
-- non-positive total body water at an age of **77.93 years** — below the 130
-- years the API accepts as a birth date. The floors raise that crossing from
-- 25.72 years to 77.93; they do not remove it. Removing it takes a guard on the
-- sign of the total body water inside `crates/domain`, which is issue #16.
--
-- The clauses of the initial migration are left where they are: migrations are
-- append-only once merged. The constraints below are added beside them, and
-- being stricter they are the ones that decide.

ALTER TABLE public.profile_settings_version
    ADD CONSTRAINT profile_settings_version_weight_kg_is_a_person
        CHECK (weight_kg > 2 AND weight_kg < 1000),
    ADD CONSTRAINT profile_settings_version_height_cm_is_a_person
        CHECK (height_cm > 40 AND height_cm < 300);

COMMENT ON CONSTRAINT profile_settings_version_weight_kg_is_a_person
    ON public.profile_settings_version IS
    'Lower bound under the lightest verified human; not a bound that makes the Watson equation of SPEC.md §6.2 well behaved.';
COMMENT ON CONSTRAINT profile_settings_version_height_cm_is_a_person
    ON public.profile_settings_version IS
    'Lower bound under the shortest verified human; not a bound that makes the Watson equation of SPEC.md §6.2 well behaved.';
