[![CI](https://github.com/maelprog/alcoLoco/actions/workflows/ci.yml/badge.svg)](https://github.com/maelprog/alcoLoco/actions/workflows/ci.yml)

# alcoLoco

Blood alcohol tracking application. Each user records what they drink — plain drinks or multi-part
cocktails — and the application computes and displays their blood alcohol concentration over time,
both individually and side by side with the other participants of a same event.

> ⚠️ **Disclaimer** — The blood alcohol concentration shown is a **statistical estimate**. It must
> never be used to decide whether someone is fit to drive.

## Architecture at a glance

```mermaid
flowchart TB
    CI["CI: format, clippy, lint, tests"]

    subgraph WEB["web/ (Angular)"]
        SHELL["Router, layout, nav shell"]
        SCREENS["Feature screens (skeleton)"]
        CLIENT["Typed API client"]
    end

    subgraph RUST["Rust workspace"]
        subgraph API["crates/api (axum)"]
            ROUTES["Health and OpenAPI routes"]
            BIZ["Business endpoints (skeleton)"]
        end
        subgraph DOMAIN["crates/domain (zero dependencies)"]
            BRICKS["Widmark and Watson bricks"]
            CURVE["Curve integration (skeleton)"]
        end
        MIG["crates/db: versioned migrations"]
    end

    PG[("PostgreSQL 16")]

    CI --> WEB
    CI --> RUST

    SHELL --> SCREENS
    SHELL -- health indicator --> CLIENT
    SCREENS -.-> CLIENT
    CLIENT -- "HTTP, RFC 7807 errors" --> ROUTES
    CLIENT -.-> BIZ
    BIZ -.-> BRICKS
    BRICKS -.-> CURVE
    BIZ -.-> PG
    ROUTES -- probe --> PG
    MIG -- owns schema --> PG

    classDef skeleton stroke-dasharray:5 5
    class SCREENS,BIZ,CURVE skeleton
```

**Solid = code on `main`. Dotted = shape agreed, not written yet (skeleton).**
`crates/domain` has zero dependencies and only inbound arrows: no database, no HTTP, no framework.

---

## Project status

The technical skeleton is in place: Rust workspace, Angular application, local PostgreSQL and CI.
**No business feature works end to end**: no endpoint serves business data and no screen calls the
API. What sits behind that headline is nonetheless more than a placeholder, and the diagram above
marks the difference — dotted is what does not exist yet.

- **`crates/db`** — the whole V1 schema, migrated and seeded: profiles and their versioned
  settings, events, drinks and components, the base library.
- **`crates/api`** — what every future endpoint shares: configuration, RFC 7807 error rendering,
  UUID v7 identifiers, cursor pagination, the generated OpenAPI document. The routes actually
  served are `GET /health` and, outside production, `GET /openapi.json`.
- **`crates/domain`** — the reference calculation bricks of §6.1 to §6.4. Assembling them into a
  curve (§6.5) is [#16](https://github.com/maelprog/alcoLoco/issues/16), and **no crate depends on
  `domain` yet**: the API does not call it.
- **`web/`** — routing, layout shell, typed API client and RFC 7807 error interceptor. The feature
  screens behind the routes are placeholders.

The single source of truth is **[docs/SPEC.md](docs/SPEC.md)**. Every question about scope, domain
model or calculation rules is settled there; this README is only an entry-point summary.

## Tech stack

| Layer | Technology |
|---|---|
| Backend | Rust — **axum** framework |
| Frontend | TypeScript — **Angular** |
| Database | **PostgreSQL** |

---

## Local setup

### Prerequisites

| Tool | Reference version | Used for |
|---|---|---|
| Docker + Docker Compose | v2 (`docker compose`) | local database |
| Rust (`rustup`) | **1.97.1**, with `rustfmt` and `clippy` | backend |
| Node.js + npm | **22.x** | frontend |

The Rust version is the one pinned by CI (`.github/workflows/ci.yml`); using a different one
locally invites `rustfmt` and `clippy` discrepancies.

### Repository layout

```
crates/api/         axum binary — HTTP entry point
crates/db/          PostgreSQL schema, migrations and development seed data
crates/domain/      calculation engine, free of I/O and framework (docs/SPEC.md §7, §9)
web/                Angular application
docs/               project documentation, including the spec (docs/SPEC.md)
docker-compose.yml  local PostgreSQL
```

### Database

```bash
docker compose up -d          # starts PostgreSQL 16
docker compose logs -f db     # follows the logs
docker compose down           # stops; add -v to wipe the data
```

The service listens on `localhost:5432` and the default credentials are `alcoloco` / `alcoloco`,
database `alcoloco`. They can be overridden through environment variables — handy to run several
instances in parallel:

```bash
POSTGRES_PORT=55432 docker compose -p alcoloco-other up -d
```

| Variable | Default |
|---|---|
| `POSTGRES_USER` | `alcoloco` |
| `POSTGRES_PASSWORD` | `alcoloco` |
| `POSTGRES_DB` | `alcoloco` |
| `POSTGRES_PORT` | `5432` |

### Migrations and development seed data

The schema lives in `crates/db/migrations/`. Migrations are **embedded at compile time** and applied
by the `db` tool, which records them in the `_sqlx_migrations` table: re-running `migrate` against
an up-to-date database replays nothing.

```bash
cargo run -p db -- migrate    # applies pending migrations
cargo run -p db -- seed       # inserts the development seed data (empty database)
cargo run -p db -- reset      # drops the public schema, re-migrates, then seeds
```

The connection string comes from `DATABASE_URL`, defaulting to
`postgres://alcoloco:alcoloco@localhost:5432/alcoloco` — the credentials from `docker-compose.yml`.

Migrations are **append-only** once merged to `main`: never edit a file that has already been
applied (the tool would reject the checksum), add a new one instead.

Schema facts worth knowing before writing a query:

- identifiers are `uuid` columns **without a default**: the application supplies a **UUID v7**
  (PostgreSQL 16 has no native `uuidv7()`);
- physiological parameters (weight, height, sex, date of birth) live **only** in
  `profile_settings_version`; `profile` carries nothing but identity and input preferences, which
  are not versioned;
- a version stores only its lower bound: the upper bound is the next `valid_from`, which makes
  overlaps and gaps **unrepresentable**;
- `profile_settings_at(profile_id, instant)` returns the version in effect at a given instant,
  falling back to the oldest one (open lower bound). It `RETURNS SETOF`, so write
  `SELECT * FROM profile_settings_at($1, $2)`, or `LEFT JOIN LATERAL … ON true` when the enclosing
  row must survive an empty result. The scalar form `(profile_settings_at($1, $2)).weight_kg` is
  either illegal or silently drops the whole enclosing row;
- durations are integer seconds (`*_duration_seconds`), volumes are millilitres (`*_ml`).

Every profile is meant to hold **at least one settings version**, and deferred constraint triggers
enforce that for a transaction running on its own. The order of the writes inside it is not free for
all that: the profile must precede its first version, the reverse order being refused **outright**
rather than at commit, since a version cannot reference a profile that does not exist yet. The
guarantee is **not absolute**: bulk wipes (`TRUNCATE`), anything that disables the triggers, and
above all **concurrent writes** can leave a profile durably versionless, with
`profile_settings_at()` returning no row for a profile that still exists and signalling nothing.
**Consequence: no feature — #7, #16 or any other — may rely on this invariant in the presence of
concurrent writes until issue [#43](https://github.com/maelprog/alcoLoco/issues/43) is closed**;
#43 carries the actual fix and blocks [#7](https://github.com/maelprog/alcoLoco/issues/7). The
exhaustive account — every escape class, who can reach it and with which privileges — is in
[§5.1](docs/SPEC.md#51-profils) and [§10.0-L](docs/SPEC.md#100-décisions-actées) of the spec.

### Backend

```bash
cargo run -p api                          # serves on http://localhost:8080
curl http://localhost:8080/health         # -> {"status":"ok","database":"up","checked_at":"…"}
curl http://localhost:8080/openapi.json   # the generated OpenAPI document
```

`GET /health` always answers `200` and reports the database separately: `status` is `degraded` and
`database` is `down` when PostgreSQL does not answer, so the process stays diagnosable while the
database is out. Every failure — including an unknown path or a wrong method — answers
`application/problem+json` (RFC 7807).

The listen address can be overridden with `ALCOLOCO_API_ADDR` (default `0.0.0.0:8080`).
`ALCOLOCO_ENV` defaults to `development`, which is what serves the OpenAPI document; set it to
`production` to withhold it.

Quality checks, identical to CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

### Frontend

```bash
cd web
npm ci
npm start                     # serves on http://localhost:4200
npm run build                 # production bundle in web/dist/
```

Quality checks, identical to CI:

```bash
npm run lint
npm run test -- --watch=false
```

Frontend unit tests run under **Vitest** with the `jsdom` environment: no browser required.

### Continuous integration

The `.github/workflows/ci.yml` workflow runs on every pull request and on `main`. It carries two
jobs: **`rust`** (format, clippy, tests) and **`web`** (lint, tests).

## V1 scope

- Predefined profiles, **no authentication** — picked from a list at startup.
- Event creation and direct addition of participating profiles (no invitations).
- Drink entry in 3 modes: **free-form entry**, **mixology menu** (a drink made of several
  components), **duplication from history**.
- A read-only base drink library shipped with the application, searched by name.
- Blood alcohol computed with the **Widmark formula**, the volume of distribution estimated from the
  **Watson** equations.
- Display of the instantaneous blood alcohol concentration (g/L of blood) and of the participants'
  curves side by side over the duration of the event.
- Tracking your own blood alcohol concentration from your profile, outside any event.
- Per-profile drink history across all events, editable and deletable.
- Versioned history of the profile's physiological parameters (weight, height, sex, **date of
  birth**), each version carrying a **user-chosen effective date**, so that correcting a mistyped
  weight and recording a real change stay two distinct gestures
  ([§10.0-J](docs/SPEC.md#100-décisions-actées)).
- An **optional** subjective rating captured at entry time, on a 1-to-10 scale, with a default
  value suggested from the computed blood alcohol concentration — the suggestion is **capped at
  level 6** and never proposes anything above it ([§10.0-E](docs/SPEC.md#100-décisions-actées)).

### Deferred to V2+

Authentication, accounts and **event invitations** · per-event "track my drinking" toggle ·
growable personal library and access to other participants' libraries · CSV export · personalised
blood alcohol profile with calibration · extraction of the mixology service into a microservice ·
integration with *Manage Our Home*.

## Domain model

- **Profile** — identity and input preferences (default quantity unit, cL or %, default ingestion
  duration, default absorption duration). It carries **no physiological parameter at all**: weight,
  height, sex and date of birth belong to the settings version in force and are read through it. A
  "current" copy on the profile would be a second source of truth, free to diverge from that
  version, and no computation would read it: every one of them is required to use the version in
  force **at the ingestion time** of the drink ([§10.0-L](docs/SPEC.md#100-décisions-actées)).
- **ProfileSettingsVersion** — a timestamped snapshot of the physiological parameters; every drink
  is attached to the version in effect at its ingestion time, so that past curves can be replayed
  faithfully.
- **Event** — a dated event and its participating profiles.
- **Drink** — a consumed drink: name, ingestion time and duration, absorption duration, optional
  subjective rating, optional event, one or more components.
- **DrinkComponent** — name, alcohol by volume (% vol), quantity in cL or as a percentage of the
  drink's total volume. The unit is chosen **per drink**; as soon as one component is expressed in
  %, the **total volume is mandatory** — without it the blood alcohol concentration cannot be
  computed and the drink is invalid ([§10.0-A](docs/SPEC.md#100-décisions-actées)).
- **LibraryItem** — an entry of the base library: name, default ABV, category (alcoholic / soft).

Key relationship: a drink **always** belongs to the profile's history; attaching it to an event is
optional.

## Calculation engine

Full detail in [§6 of the spec](docs/SPEC.md#6-règles-de-calcul-de-référence).

- **Ingested alcohol**: `A (g) = volume(mL) × abv(%) / 100 × 0.789`, summed over the components; a
  component expressed in % is first converted to a volume through the drink's total volume.
- **Volume of distribution**: total body water (TBW) from **Watson (1980)** using weight, height,
  age and sex; `C₀ = 0.806 × A / TBW`. Falls back to the Widmark constants (`r` = 0.68 / 0.55) when
  a datum is missing. Age is the age at the drink's ingestion time.
- **Elimination**: zero-order, `β = 0.15 g/L/h` by default (literature: 0.10 – 0.20), **clamped at
  zero** — no debt carried over to a later drink. In V1 `β` is a **constant of the `domain` crate**,
  exposed as a function argument for the tests: neither a profile field nor a database column,
  per-profile calibration being deferred to V2+ ([§10.0-H](docs/SPEC.md#100-décisions-actées)).
- **Absorption profile**: **trapezoid** (convolution of ingestion duration with absorption
  duration), adopted as the *implementation default*. It degenerates into the linear ramp and into
  pure Widmark depending on the entered durations.
- **Superposition**: appearance rates add up, but the elimination term `β` is global to the body and
  applies **only once** — summing individual curves that each carry their own `− β × t` would
  eliminate at `N × β`, and that is a correctness constraint, not an option. The curve is obtained
  by forward integration at a fixed **internal step of 1 min**
  ([§10.3](docs/SPEC.md#103-granularité-des-courbes--acté-le-2026-08-17-voir-100-i)).

The computation is **deterministic and replayable**: identical history, identical curve. The engine
lives in `crates/domain`, a crate with **no dependency at all** — no I/O, no database, no framework
— so it is unit-testable in isolation and extractable later (§7, §9).

Written today: the bricks of §6.1 to §6.4 — ingested alcohol, Watson body water and `C₀`,
elimination, trapezoidal absorption — with the reference cases of #5 pinned by tests. Not written:
the superposition and the fixed-step integration of §6.5, which are
[#16](https://github.com/maelprog/alcoLoco/issues/16). No crate calls `domain` yet.

> 🚧 The choice of absorption profile is **not final**: it must be compared against the alternatives
> (pure Widmark, linear ramp, first-order exponential) **before the V1 release** — see
> [§10.1](docs/SPEC.md#101-modèle-dingestion-et-dabsorption--défaut-retenu-arbitrage-final-avant-release-v1-37)
> and issue [#37](https://github.com/maelprog/alcoLoco/issues/37).

## Architecture

V1 is a monolith, but it is carved up to anticipate three microservice extractions: **mixology /
library**, **groups & events** (to be delegated to *Manage Our Home*) and the **blood alcohol
engine** (a pure, stateless computation service). Concretely: separate Rust modules, explicit
boundaries, and no direct access from one module to another module's tables.

Other cross-cutting constraints: timestamps are stored in UTC and displayed in the client's local
time zone.

## Open questions

| Topic | Status |
|---|---|
| Final arbitration of the absorption model | V1 blocker — issue #37 |
| Persistence of off-library components in V1 | To be confirmed ([§10.2](docs/SPEC.md#102-composantes-hors-bibliothèque-en-v1--ouvert)) |
| Integration step and display window of the curves | Settled on 2026-08-17 — internal step of 1 min, never exposed; window = event duration + 3 h. The API's `?step=` only subsamples the integrated series and is bounded to [1 min, 1 h], so it can never change the result ([§10.3](docs/SPEC.md#103-granularité-des-courbes--acté-le-2026-08-17-voir-100-i)) |
| Adding an item to the library | Deferred to V2+ ([§10.4](docs/SPEC.md#104-ajout-dun-item-à-la-bibliothèque-v2--ouvert)) |

## License

Released under the [MIT License](LICENSE).
