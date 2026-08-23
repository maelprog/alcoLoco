//! Every statement the profile endpoints run.
//!
//! Three rules hold across the file, and they are the reason it exists as a
//! module of its own rather than as SQL inlined in the handlers.
//!
//! **No value is ever formatted into a statement.** Every statement below is a
//! `&'static str`; user input reaches PostgreSQL through `bind` and only through
//! `bind`. The seed of the `db` crate builds two of its statements with
//! `format!` — safely, since nothing external reaches them — and this module
//! deliberately does not take up that pattern: it is the first code of the
//! project to write values a client sent.
//!
//! **Relations and types are schema qualified.** `pg_temp` is searched before
//! `public`, so an unqualified `profile` can be shadowed by a temporary table
//! (class (e) of the migration of #2). Qualifying costs nothing and closes it.
//!
//! **The settings come from `profile_settings_at`, never from a column of
//! `profile`.** There is no current copy to read: SPEC.md §10.0-L puts the four
//! physiological parameters in `profile_settings_version` and only there. The
//! function returns zero or one row, so it is joined with `LEFT JOIN LATERAL …
//! ON true` — a plain join would drop the profile along with its missing
//! settings, and the scalar spelling `(profile_settings_at(…)).weight_kg` would
//! remove the whole row of the enclosing query.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::ApiError;
use crate::id::new_id;
use crate::pagination::{Page, PageRequest};
use crate::timestamp::Timestamp;

use super::model::{Profile, ProfileSettings, QuantityUnit, Sex};
use super::validation::{ValidProfile, ValidSettings};

/// Constraint that refuses two versions of a profile starting at the same
/// instant. Named by the migration of #2; a violation of it is the caller's
/// fault, not the server's, so it answers 409 rather than 500.
const UNIQUE_VERSION_START: &str = "profile_settings_version_unique_start";

/// One page of profiles, resumed after `$1` when it is not null.
///
/// `$1` is cast because a bound `Option<Uuid>` arrives as an untyped null, which
/// PostgreSQL cannot compare. Ordering by `id` is ordering by creation: the
/// identifiers are UUID v7 (see [`crate::pagination`]).
const SELECT_PAGE: &str = "
    SELECT p.id,
           p.display_name,
           p.default_quantity_unit::text AS default_quantity_unit,
           p.default_ingestion_duration_seconds,
           p.default_absorption_duration_seconds,
           p.created_at,
           p.updated_at,
           s.valid_from,
           s.weight_kg,
           s.height_cm,
           s.sex::text AS sex,
           s.birth_date
    FROM public.profile AS p
    LEFT JOIN LATERAL public.profile_settings_at(p.id, now()) AS s ON true
    WHERE $1::uuid IS NULL OR p.id > $1::uuid
    ORDER BY p.id
    LIMIT $2
";

/// One profile with the settings in force now.
const SELECT_ONE: &str = "
    SELECT p.id,
           p.display_name,
           p.default_quantity_unit::text AS default_quantity_unit,
           p.default_ingestion_duration_seconds,
           p.default_absorption_duration_seconds,
           p.created_at,
           p.updated_at,
           s.valid_from,
           s.weight_kg,
           s.height_cm,
           s.sex::text AS sex,
           s.birth_date
    FROM public.profile AS p
    LEFT JOIN LATERAL public.profile_settings_at(p.id, now()) AS s ON true
    WHERE p.id = $1
";

/// The identity and preferences half of a profile.
const INSERT_PROFILE: &str = "
    INSERT INTO public.profile (
        id, display_name, default_quantity_unit,
        default_ingestion_duration_seconds, default_absorption_duration_seconds
    ) VALUES ($1, $2, $3::public.quantity_unit, $4, $5)
";

/// A settings version. Identifiers are minted by the API: PostgreSQL 16 has no
/// native `uuidv7()` and no column carries a default.
const INSERT_VERSION: &str = "
    INSERT INTO public.profile_settings_version (
        id, profile_id, valid_from, weight_kg, height_cm, sex, birth_date
    ) VALUES ($1, $2, $3, $4, $5, $6::public.sex, $7)
";

/// The preferences half of an update. The physiological half never travels here:
/// it is a new row of `profile_settings_version` or nothing at all.
const UPDATE_PROFILE: &str = "
    UPDATE public.profile
    SET display_name = $2,
        default_quantity_unit = $3::public.quantity_unit,
        default_ingestion_duration_seconds = $4,
        default_absorption_duration_seconds = $5
    WHERE id = $1
";

/// Takes the row lock that serialises two updates of the same profile.
const LOCK_PROFILE: &str = "SELECT 1 FROM public.profile WHERE id = $1 FOR UPDATE";

/// The parameters in force at a given instant, for the comparison that decides
/// whether an update has to post a version at all (SPEC.md §10.0-J).
const SELECT_SETTINGS_AT: &str = "
    SELECT valid_from, weight_kg, height_cm, sex::text AS sex, birth_date
    FROM public.profile_settings_at($1, $2)
";

/// Reads a row of [`SELECT_PAGE`] or [`SELECT_ONE`].
///
/// A label the `sex` or `quantity_unit` enum carries but the API does not know
/// is an internal failure, not a missing profile: the schema and the code have
/// diverged, and answering 404 would hide that behind a plausible-looking
/// nothing.
fn profile_from_row(row: &PgRow) -> Result<Profile, ApiError> {
    let unit_label: String = row.try_get("default_quantity_unit")?;
    let unit = QuantityUnit::from_sql(&unit_label)
        .ok_or_else(|| ApiError::internal(format!("unknown quantity_unit `{unit_label}`")))?;

    Ok(Profile {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        default_quantity_unit: unit,
        default_ingestion_duration_seconds: row.try_get("default_ingestion_duration_seconds")?,
        default_absorption_duration_seconds: row.try_get("default_absorption_duration_seconds")?,
        settings: settings_from_row(row)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Reads the settings columns of a row, which are all null together when the
/// lateral join found no applicable version.
fn settings_from_row(row: &PgRow) -> Result<Option<ProfileSettings>, ApiError> {
    let Some(valid_from) = row.try_get::<Option<Timestamp>, _>("valid_from")? else {
        return Ok(None);
    };
    let sex_label: String = row.try_get("sex")?;
    let sex = Sex::from_sql(&sex_label)
        .ok_or_else(|| ApiError::internal(format!("unknown sex `{sex_label}`")))?;

    Ok(Some(ProfileSettings {
        valid_from,
        weight_kg: row.try_get("weight_kg")?,
        height_cm: row.try_get("height_cm")?,
        sex,
        birth_date: row.try_get("birth_date")?,
    }))
}

/// Reads one page of profiles, in identifier order.
///
/// # Errors
///
/// Fails when the query cannot be run or a row cannot be read.
pub async fn list(pool: &PgPool, request: PageRequest) -> Result<Page<Profile>, ApiError> {
    let rows = sqlx::query(SELECT_PAGE)
        .bind(request.cursor)
        .bind(request.fetch_limit())
        .fetch_all(pool)
        .await?;

    let profiles = rows
        .iter()
        .map(profile_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Page::from_rows(profiles, request, |profile| profile.id))
}

/// Reads one profile, or nothing when no profile carries that identifier.
///
/// # Errors
///
/// Fails when the query cannot be run or the row cannot be read.
pub async fn read(pool: &PgPool, id: Uuid) -> Result<Option<Profile>, ApiError> {
    sqlx::query(SELECT_ONE)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(profile_from_row)
        .transpose()
}

/// Writes a settings version inside an open transaction.
async fn insert_version(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: Uuid,
    settings: &ValidSettings,
) -> Result<(), sqlx::Error> {
    sqlx::query(INSERT_VERSION)
        .bind(new_id())
        .bind(profile_id)
        .bind(settings.valid_from)
        .bind(settings.weight_kg)
        .bind(settings.height_cm)
        .bind(settings.sex.as_sql())
        .bind(settings.birth_date)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Turns a duplicate `valid_from` into the caller-side failure it is.
fn as_api_error(error: sqlx::Error) -> ApiError {
    let is_duplicate_start = matches!(&error,
        sqlx::Error::Database(database) if database.constraint() == Some(UNIQUE_VERSION_START));
    if is_duplicate_start {
        return ApiError::conflict("a settings version already starts at that `valid_from`");
    }
    ApiError::from(error)
}

/// Creates a profile and its first settings version.
///
/// Both rows are written in one transaction, profile first: the deferred
/// constraint trigger of the schema checks at `COMMIT` that the profile owns a
/// version, and the foreign key of the version refuses the reverse order on the
/// spot (SPEC.md §5.1).
///
/// # Errors
///
/// Answers a conflict when a version already starts at that instant, and fails
/// on any other write error.
pub async fn create(pool: &PgPool, id: Uuid, profile: &ValidProfile) -> Result<(), ApiError> {
    let mut tx = pool.begin().await?;

    sqlx::query(INSERT_PROFILE)
        .bind(id)
        .bind(&profile.display_name)
        .bind(profile.default_quantity_unit.as_sql())
        .bind(profile.default_ingestion_duration_seconds)
        .bind(profile.default_absorption_duration_seconds)
        .execute(&mut *tx)
        .await
        .map_err(as_api_error)?;

    insert_version(&mut tx, id, &profile.settings)
        .await
        .map_err(as_api_error)?;

    tx.commit().await.map_err(as_api_error)
}

/// What an update did, so that the handler can answer the right status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Updated {
    /// No profile carries that identifier.
    NoSuchProfile,
    /// The preferences were written; the parameters were already those in force.
    PreferencesOnly,
    /// The preferences were written and a settings version was posted.
    WithNewVersion,
}

/// Replaces a profile, posting a settings version only if one is needed.
///
/// A version is posted when the four parameters of SPEC.md §10.0-D differ from
/// those in force **at the requested `valid_from`**, and when the profile owns no
/// applicable version at all. Rewriting only the input preferences posts nothing:
/// they enter no computation, and SPEC.md §10.0-J leaves them unversioned.
///
/// The comparison is made against the instant the caller asked for rather than
/// against now, so that a retroactive correction is judged against the values it
/// actually corrects. Closing the versions already later than `valid_from` is the
/// remaining half of SPEC.md §5.1 and belongs to issue #7.
///
/// # Errors
///
/// Answers a conflict when a version already starts at that instant, and fails
/// on any other write error.
pub async fn update(pool: &PgPool, id: Uuid, profile: &ValidProfile) -> Result<Updated, ApiError> {
    let mut tx = pool.begin().await?;

    // Taken before anything is read, so that two updates of the same profile
    // cannot both decide "no version needed" from the same stale reading.
    let locked = sqlx::query(LOCK_PROFILE)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if locked.is_none() {
        return Ok(Updated::NoSuchProfile);
    }

    sqlx::query(UPDATE_PROFILE)
        .bind(id)
        .bind(&profile.display_name)
        .bind(profile.default_quantity_unit.as_sql())
        .bind(profile.default_ingestion_duration_seconds)
        .bind(profile.default_absorption_duration_seconds)
        .execute(&mut *tx)
        .await
        .map_err(as_api_error)?;

    let in_force = sqlx::query(SELECT_SETTINGS_AT)
        .bind(id)
        .bind(profile.settings.valid_from)
        .fetch_optional(&mut *tx)
        .await?
        .as_ref()
        .map(settings_from_row)
        .transpose()?
        .flatten();

    let needs_version = in_force
        .as_ref()
        .is_none_or(|current| profile.settings.differs_from(current));

    if needs_version {
        insert_version(&mut tx, id, &profile.settings)
            .await
            .map_err(as_api_error)?;
    }

    tx.commit().await.map_err(as_api_error)?;

    Ok(if needs_version {
        Updated::WithNewVersion
    } else {
        Updated::PreferencesOnly
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every statement this module runs.
    const STATEMENTS: [&str; 7] = [
        SELECT_PAGE,
        SELECT_ONE,
        INSERT_PROFILE,
        INSERT_VERSION,
        UPDATE_PROFILE,
        LOCK_PROFILE,
        SELECT_SETTINGS_AT,
    ];

    /// The names `keyword` introduces, `keyword` being given with its trailing
    /// space and matched on an upper-cased copy of the statement.
    ///
    /// `LATERAL` is skipped rather than returned: `JOIN LATERAL public.f(…)`
    /// names `public.f`, and the `LATERAL ` keyword picks it up on its own pass.
    fn names_after(statement: &str, keyword: &str) -> Vec<String> {
        let upper = statement.to_uppercase();
        let mut names = Vec::new();
        let mut from = 0;
        while let Some(at) = upper[from..].find(keyword) {
            let after = from + at + keyword.len();
            let name: String = statement[after..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !name.eq_ignore_ascii_case("LATERAL") && !name.is_empty() {
                names.push(name);
            }
            from = after;
        }
        names
    }

    #[test]
    fn every_name_in_relation_position_is_schema_qualified() {
        // `pg_temp` is searched before `public`, so an unqualified name can be
        // shadowed by a temporary table — the hole found on #41, where it turned
        // `profile_settings_at()` into a source of fabricated parameters. Only
        // names in relation position are looked at: a column called `sex` is not
        // one, and neither is the `profile` inside `profile_id`.
        for statement in STATEMENTS {
            for keyword in ["FROM ", "JOIN ", "LATERAL ", "INTO ", "UPDATE "] {
                for name in names_after(statement, keyword) {
                    assert!(
                        name.starts_with("public."),
                        "`{name}` follows `{}` unqualified in: {statement}",
                        keyword.trim()
                    );
                }
            }
        }
    }

    #[test]
    fn every_cast_to_a_type_this_schema_owns_is_schema_qualified() {
        // A cast resolves through `search_path` too, so `$3::quantity_unit` is
        // shadowable by a temporary type. Built-in types are not: `pg_catalog`
        // comes first whatever `search_path` says.
        const BUILT_IN: [&str; 2] = ["uuid", "text"];
        for statement in STATEMENTS {
            for name in names_after(statement, "::") {
                assert!(
                    name.starts_with("public.") || BUILT_IN.contains(&name.as_str()),
                    "the cast to `{name}` is unqualified in: {statement}"
                );
            }
        }
    }

    #[test]
    fn no_statement_of_this_module_is_built_from_a_value() {
        // The pattern the journal of #6 asks this issue not to take up from the
        // seed of #2. Every statement is a literal and every value is bound, so
        // each one has to carry at least one placeholder or none at all — never
        // an interpolation marker left behind by a `format!`.
        for statement in STATEMENTS {
            assert!(
                !statement.contains('{') && !statement.contains('}'),
                "a formatting placeholder survived into: {statement}"
            );
        }
    }

    #[test]
    fn the_settings_of_a_profile_are_read_through_the_versioned_function() {
        // Guards the arbitration of 2026-08-19: there is no current copy on
        // `profile` to read, so a query selecting `p.weight_kg` would not even
        // compile in PostgreSQL — but a *new* one added later might reach for a
        // column added later. Both read paths go through the function.
        for statement in [SELECT_PAGE, SELECT_ONE] {
            assert!(
                statement.contains("public.profile_settings_at(p.id, now())"),
                "the settings are not read from the versioned function: {statement}"
            );
            assert!(
                statement.contains("LEFT JOIN LATERAL"),
                "an inner join would drop a profile that owns no version: {statement}"
            );
        }
        assert!(SELECT_SETTINGS_AT.contains("public.profile_settings_at($1, $2)"));
    }
}
