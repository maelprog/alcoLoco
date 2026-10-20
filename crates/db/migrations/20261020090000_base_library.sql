-- Base library: search key, search index and initial data set (issue #10,
-- SPEC.md §5.4).
--
-- The table itself comes from the initial migration; this one makes it
-- searchable and fills it. The library is shipped with the application and
-- read-only in V1, so its rows belong in a migration: every database the
-- application runs against holds the same library, with the same identifiers.

-- ---------------------------------------------------------------------------
-- Search key
-- ---------------------------------------------------------------------------
--
-- The search of SPEC.md §5.4 must find « bière » when « biere » is typed, so
-- names are compared through a key that ignores case and accents:
-- `unaccent`, then `lower`. `unaccent` is a contrib extension shipped with
-- PostgreSQL (and with the `postgres:16-alpine` image of docker-compose.yml);
-- it is marked trusted since PostgreSQL 13, so a role holding CREATE on the
-- database may install it without being superuser.
--
-- Both forms of `unaccent` are declared STABLE (`provolatile = 's'` in
-- `pg_proc`), the two-argument one included: what they return depends on a
-- text search dictionary, whose rules file and options can change while the
-- server runs, and not on their arguments alone. The one-argument form does
-- not depend on `search_path` for it: it takes the `unaccent` dictionary of
-- its own schema. Measured on PostgreSQL 16: `public.unaccent('bière')`
-- returns `biere` with `search_path` set to `pg_catalog`, to `pg_temp,
-- pg_catalog`, or to the empty string.
--
-- Declaring the key below IMMUTABLE, which an index requires, is therefore a
-- promise this migration makes, not one `unaccent` makes. Its share of the
-- promise: the key names its dictionary, and the SQL-standard body binds every
-- name when the function is created, so no later `search_path` — `pg_temp`
-- included — changes what it resolves to. What it cannot promise is that the
-- dictionary's rules and the collation `lower` runs under stay as they are:
-- changing either on a live server calls for a REINDEX of the index below,
-- like any expression index over text.

CREATE EXTENSION IF NOT EXISTS unaccent WITH SCHEMA public;

CREATE FUNCTION public.library_item_search_key(name pg_catalog.text)
    RETURNS pg_catalog.text
    LANGUAGE sql
    IMMUTABLE
    STRICT
    PARALLEL SAFE
RETURN pg_catalog.lower(
    public.unaccent('public.unaccent'::pg_catalog.regdictionary, name)
);

COMMENT ON FUNCTION public.library_item_search_key(pg_catalog.text) IS
    'Case- and accent-insensitive key of a library name (SPEC.md §5.4). Search with library_item_search_key(name) LIKE library_item_search_key(<typed>) || ''%''.';

-- ---------------------------------------------------------------------------
-- Search index, and the scope of a name's uniqueness
-- ---------------------------------------------------------------------------
--
-- The initial migration made `lower(name)` unique across the whole library,
-- every category together, without saying whether that scope was meant. It
-- is: a library name identifies an item for whoever searches it. The search
-- shows names, and a drink component copies the name and the degree of the
-- item it comes from, never its category (SPEC.md §4, §5.3) — so a soft and an
-- alcohol sharing a name would reach the user as two indistinguishable rows
-- with different degrees.
--
-- For the same reason the uniqueness now follows the search key rather than
-- `lower(name)` alone: « Bière blonde » and « Biere blonde » are as much the
-- same entry, to the search, as « Vodka » and « vodka ». The index enforcing it
-- is also the search index. `text_pattern_ops` is what lets it answer the
-- prefix search `key LIKE 'biere%'` under the database's non-C collation; the
-- default operator class would only serve equality there.
--
-- The initial `library_item_lower_name_key` stays, and it is not redundant
-- with this one: neither index implies the other. `unaccent` maps some
-- characters to a letter that `lower` alone would not reach, and the reverse.
-- The ångström sign (U+212B) and « å » have the same `lower()`, « å », but
-- different keys here — « å » for the first, « a » for the second. That pair
-- is refused by the initial index only.

CREATE UNIQUE INDEX library_item_search_key_unique
    ON public.library_item (public.library_item_search_key(name) pg_catalog.text_pattern_ops);

-- ---------------------------------------------------------------------------
-- Initial data set
-- ---------------------------------------------------------------------------
--
-- Common alcohols with a usual degree, and common soft drinks at 0 % (SPEC.md
-- §5.4). The degree is a default: it pre-fills the component and the user
-- corrects it for the bottle in hand (SPEC.md §5.3, mode 2). Generic names
-- only, no brand.
--
-- PostgreSQL 16 has no native uuidv7(), and the project generates identifiers
-- on the application side (SPEC.md §10.0); rows written by a migration
-- therefore carry literal UUID v7. They all share the timestamp of this
-- migration's version, 2026-10-20T09:00:00Z, and number the rows in their
-- 12-bit `rand_a` field, so they sort in the order of this list.

INSERT INTO public.library_item (id, name, category, default_abv_percent) VALUES
    ('01a15889-fa80-7000-a8ef-f3cd82e3fa2f', 'Vodka', 'alcohol', 40),
    ('01a15889-fa80-7001-a82e-3a31b2fae7a4', 'Gin', 'alcohol', 40),
    ('01a15889-fa80-7002-82c5-059cc38d13b0', 'Rhum blanc', 'alcohol', 40),
    ('01a15889-fa80-7003-a30e-eb9e12779f02', 'Rhum ambré', 'alcohol', 40),
    ('01a15889-fa80-7004-9e5d-4afe4714f8fe', 'Whisky', 'alcohol', 40),
    ('01a15889-fa80-7005-9272-96f35f437087', 'Bourbon', 'alcohol', 40),
    ('01a15889-fa80-7006-833b-4f7d1bc362eb', 'Tequila', 'alcohol', 38),
    ('01a15889-fa80-7007-a60d-9fba8eaa7e34', 'Mezcal', 'alcohol', 40),
    ('01a15889-fa80-7008-9572-4bb9ab03dc1c', 'Cachaça', 'alcohol', 40),
    ('01a15889-fa80-7009-afe1-37c18a03a40d', 'Cognac', 'alcohol', 40),
    ('01a15889-fa80-700a-a77a-de36cc3dc70b', 'Armagnac', 'alcohol', 40),
    ('01a15889-fa80-700b-bfb7-c558da78011b', 'Calvados', 'alcohol', 40),
    ('01a15889-fa80-700c-a14d-35522df4ba57', 'Pastis', 'alcohol', 45),
    ('01a15889-fa80-700d-939b-559986d70897', 'Triple sec', 'alcohol', 40),
    ('01a15889-fa80-700e-bfe7-b45e9cc8d9c7', 'Amaretto', 'alcohol', 28),
    ('01a15889-fa80-700f-adc8-96cae279fea0', 'Limoncello', 'alcohol', 30),
    ('01a15889-fa80-7010-9872-a99bd20fdc6e', 'Liqueur de café', 'alcohol', 20),
    ('01a15889-fa80-7011-87c4-dbe381cb1d78', 'Crème de cassis', 'alcohol', 16),
    ('01a15889-fa80-7012-896c-f67edeba989f', 'Vermouth', 'alcohol', 16),
    ('01a15889-fa80-7013-a8a9-71f4d1a008ac', 'Porto', 'alcohol', 19),
    ('01a15889-fa80-7014-8d20-55b6f3cf5d75', 'Saké', 'alcohol', 15),
    ('01a15889-fa80-7015-bb5a-4704b56eb9c2', 'Bière blonde', 'alcohol', 5),
    ('01a15889-fa80-7016-a2a3-b81782a42d3e', 'Bière blanche', 'alcohol', 5),
    ('01a15889-fa80-7017-8006-ff21d737e844', 'Bière ambrée', 'alcohol', 6),
    ('01a15889-fa80-7018-90b7-1371bd7ecba9', 'Cidre', 'alcohol', 4.5),
    ('01a15889-fa80-7019-8641-30dc66d9e06b', 'Vin rouge', 'alcohol', 13),
    ('01a15889-fa80-701a-8844-5eed291bb21e', 'Vin blanc', 'alcohol', 12),
    ('01a15889-fa80-701b-b37b-b0f497c8108e', 'Vin rosé', 'alcohol', 12.5),
    ('01a15889-fa80-701c-a9c3-3f0782594b79', 'Champagne', 'alcohol', 12),
    ('01a15889-fa80-701d-8ed8-5c168e2a397b', 'Crémant', 'alcohol', 12),
    ('01a15889-fa80-701e-99ce-b1895cc8f7aa', 'Prosecco', 'alcohol', 11),
    ('01a15889-fa80-701f-a8df-4a5424f324e4', 'Jus d''orange', 'soft', 0),
    ('01a15889-fa80-7020-881e-1b8f88194a57', 'Jus de pomme', 'soft', 0),
    ('01a15889-fa80-7021-9702-dc40d94e3dc5', 'Jus d''ananas', 'soft', 0),
    ('01a15889-fa80-7022-a7f2-784b9c4b052a', 'Jus de pamplemousse', 'soft', 0),
    ('01a15889-fa80-7023-ba97-b312c29c57c2', 'Jus de canneberge', 'soft', 0),
    ('01a15889-fa80-7024-ac12-5f80fae4fb91', 'Jus de citron', 'soft', 0),
    ('01a15889-fa80-7025-a51d-63f6880f647e', 'Jus de citron vert', 'soft', 0),
    ('01a15889-fa80-7026-a761-1c7a85693290', 'Coca', 'soft', 0),
    ('01a15889-fa80-7027-b3c0-52cd30dfd63c', 'Limonade', 'soft', 0),
    ('01a15889-fa80-7028-9e3f-3583df51df43', 'Tonic', 'soft', 0),
    ('01a15889-fa80-7029-a96c-aff2b443423e', 'Ginger ale', 'soft', 0),
    ('01a15889-fa80-702a-a589-890e909350ac', 'Ginger beer', 'soft', 0),
    ('01a15889-fa80-702b-88e8-f38d0b660177', 'Eau gazeuse', 'soft', 0),
    ('01a15889-fa80-702c-bf2c-ae550136c71a', 'Eau plate', 'soft', 0),
    ('01a15889-fa80-702d-80ef-5a0d2ca96764', 'Sirop de sucre', 'soft', 0),
    ('01a15889-fa80-702e-b76b-cd27dc902054', 'Sirop de grenadine', 'soft', 0),
    ('01a15889-fa80-702f-8f9d-deba2e9d7ee1', 'Sirop de menthe', 'soft', 0),
    ('01a15889-fa80-7030-947d-70962c044da2', 'Sirop d''orgeat', 'soft', 0),
    ('01a15889-fa80-7031-8dc8-e84c6fdc5281', 'Lait de coco', 'soft', 0),
    ('01a15889-fa80-7032-9ddd-791ebfd44575', 'Boisson énergisante', 'soft', 0);
