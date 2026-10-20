//! Every statement the drink endpoints run.
//!
//! The rules of `profile::store` hold here too: every statement is a named
//! `&'static str` and values reach PostgreSQL through `bind` only; relations
//! and types are schema qualified.
//!
//! A drink and its components are written in one transaction, drink first.
//! The structural rules of the composition — one or more components, at most
//! 100 % in percent — are checked again by the database at `COMMIT`
//! (migration `20261021090000`), whatever the order of the writes; replacing
//! the components of a drink therefore deletes them all and inserts the new
//! ones, the drink being empty in between.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};
use crate::id::new_id;
use crate::profile::QuantityUnit;

use super::model::{Component, Drink};
use super::validation::{Defaults, ValidDrink};

/// Foreign key tying a drink attached to an event to a participant of that
/// event (migration of #2). Its violation is the caller's fault: the event
/// does not exist, or the profile does not take part in it.
const EVENT_PARTICIPANT_KEY: &str = "drink_belongs_to_an_event_participant";

/// The input preferences a creation falls back on. `FOR KEY SHARE` keeps the
/// profile from being deleted before the drink referencing it commits.
const SELECT_PROFILE_DEFAULTS: &str = "
    SELECT default_ingestion_duration_seconds, default_absorption_duration_seconds
    FROM public.profile
    WHERE id = $1
    FOR KEY SHARE
";

/// The drink row.
const INSERT_DRINK: &str = "
    INSERT INTO public.drink (
        id, profile_id, event_id, name, ingested_at,
        ingestion_duration_seconds, absorption_duration_seconds,
        quantity_unit, volume_total_ml, feeling_rating
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::public.quantity_unit, $9, $10)
";

/// One component, at its position in the drink.
const INSERT_COMPONENT: &str = "
    INSERT INTO public.drink_component (id, drink_id, sort_order, name, abv_percent, quantity)
    VALUES ($1, $2, $3, $4, $5, $6)
";

/// One drink, without its components.
const SELECT_DRINK: &str = "
    SELECT id, profile_id, event_id, name, ingested_at,
           ingestion_duration_seconds, absorption_duration_seconds,
           quantity_unit::pg_catalog.text AS quantity_unit,
           volume_total_ml, feeling_rating, created_at, updated_at
    FROM public.drink
    WHERE id = $1
";

/// [`SELECT_DRINK`], locking the row so that two modifications of the same
/// drink cannot both merge onto the same stored state.
const SELECT_DRINK_FOR_UPDATE: &str = "
    SELECT id, profile_id, event_id, name, ingested_at,
           ingestion_duration_seconds, absorption_duration_seconds,
           quantity_unit::pg_catalog.text AS quantity_unit,
           volume_total_ml, feeling_rating, created_at, updated_at
    FROM public.drink
    WHERE id = $1
    FOR UPDATE
";

/// The components of a drink, in the order they were added.
const SELECT_COMPONENTS: &str = "
    SELECT name, abv_percent, quantity
    FROM public.drink_component
    WHERE drink_id = $1
    ORDER BY sort_order
";

/// Every member of a drink but its identity and its profile.
const UPDATE_DRINK: &str = "
    UPDATE public.drink
    SET event_id = $2,
        name = $3,
        ingested_at = $4,
        ingestion_duration_seconds = $5,
        absorption_duration_seconds = $6,
        quantity_unit = $7::public.quantity_unit,
        volume_total_ml = $8,
        feeling_rating = $9
    WHERE id = $1
";

/// Every component of a drink, before the new composition is written.
const DELETE_COMPONENTS: &str = "DELETE FROM public.drink_component WHERE drink_id = $1";

/// A drink; its components go with it (`ON DELETE CASCADE`).
const DELETE_DRINK: &str = "DELETE FROM public.drink WHERE id = $1";

/// Turns a write error into the caller-side failure it may be.
fn as_api_error(error: sqlx::Error) -> ApiError {
    let names_no_participation = matches!(&error,
        sqlx::Error::Database(database) if database.constraint() == Some(EVENT_PARTICIPANT_KEY));
    if names_no_participation {
        return ApiError::validation(vec![FieldError::new(
            "event_id",
            "must name an event the profile takes part in",
        )]);
    }
    ApiError::from(error)
}

/// Reads a row of [`SELECT_DRINK`], components aside.
fn drink_from_row(row: &PgRow, components: Vec<Component>) -> Result<Drink, ApiError> {
    let unit_label: String = row.try_get("quantity_unit")?;
    let quantity_unit = QuantityUnit::from_sql(&unit_label)
        .ok_or_else(|| ApiError::internal(format!("unknown quantity_unit `{unit_label}`")))?;
    Ok(Drink {
        id: row.try_get("id")?,
        profile_id: row.try_get("profile_id")?,
        event_id: row.try_get("event_id")?,
        name: row.try_get("name")?,
        ingested_at: row.try_get("ingested_at")?,
        ingestion_duration_seconds: row.try_get("ingestion_duration_seconds")?,
        absorption_duration_seconds: row.try_get("absorption_duration_seconds")?,
        quantity_unit,
        volume_total_ml: row.try_get("volume_total_ml")?,
        feeling_rating: row.try_get("feeling_rating")?,
        components,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Reads a drink with its components inside a transaction.
async fn fetch(
    tx: &mut Transaction<'_, Postgres>,
    statement: &'static str,
    id: Uuid,
) -> Result<Option<Drink>, ApiError> {
    let Some(row) = sqlx::query(statement)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Ok(None);
    };
    let components = sqlx::query(SELECT_COMPONENTS)
        .bind(id)
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(|row| {
            Ok(Component {
                name: row.try_get("name")?,
                abv_percent: row.try_get("abv_percent")?,
                quantity: row.try_get("quantity")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    drink_from_row(&row, components).map(Some)
}

/// A duration the domain bounded to the `integer` columns, as bound.
fn seconds(value: i64) -> Result<i32, ApiError> {
    i32::try_from(value)
        .map_err(|_| ApiError::internal(format!("duration {value} escaped the domain bounds")))
}

/// Writes the components of a validated drink, in their order.
async fn insert_components(
    tx: &mut Transaction<'_, Postgres>,
    drink_id: Uuid,
    valid: &ValidDrink,
) -> Result<(), ApiError> {
    for (position, component) in valid.drink.components().iter().enumerate() {
        let sort_order = i32::try_from(position)
            .map_err(|_| ApiError::internal("more components than an integer counts"))?;
        sqlx::query(INSERT_COMPONENT)
            .bind(new_id())
            .bind(drink_id)
            .bind(sort_order)
            .bind(&component.name)
            .bind(component.abv_percent)
            .bind(component.quantity)
            .execute(&mut **tx)
            .await
            .map_err(as_api_error)?;
    }
    Ok(())
}

/// Reads one drink, or nothing when no drink carries that identifier.
///
/// # Errors
///
/// Fails when the query cannot be run or a row cannot be read.
pub async fn read(pool: &PgPool, id: Uuid) -> Result<Option<Drink>, ApiError> {
    let mut tx = pool.begin().await?;
    let drink = fetch(&mut tx, SELECT_DRINK, id).await?;
    tx.commit().await?;
    Ok(drink)
}

/// What a write found, so that the handler can answer the right status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// The profile or drink addressed does not exist.
    NotFound,
    /// The drink was written.
    Done,
}

/// Records a drink in the history of a profile.
///
/// The request is read here, inside the transaction, because its defaults
/// are the input preferences of the profile (SPEC.md §5.3).
///
/// # Errors
///
/// Answers a validation failure for an invalid drink or an event the profile
/// does not take part in, and fails on any other write error.
pub async fn create(
    pool: &PgPool,
    id: Uuid,
    profile_id: Uuid,
    read: impl FnOnce(Defaults) -> Result<ValidDrink, ApiError>,
) -> Result<Written, ApiError> {
    let mut tx = pool.begin().await?;

    let Some(row) = sqlx::query(SELECT_PROFILE_DEFAULTS)
        .bind(profile_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(Written::NotFound);
    };
    let valid = read(Defaults {
        ingestion_duration_seconds: row.try_get("default_ingestion_duration_seconds")?,
        absorption_duration_seconds: row.try_get("default_absorption_duration_seconds")?,
    })?;

    sqlx::query(INSERT_DRINK)
        .bind(id)
        .bind(profile_id)
        .bind(valid.event_id)
        .bind(valid.drink.name())
        .bind(valid.ingested_at)
        .bind(seconds(valid.drink.ingestion_duration_seconds())?)
        .bind(seconds(valid.drink.absorption_duration_seconds())?)
        .bind(QuantityUnit::from(valid.drink.unit()).as_sql())
        .bind(valid.drink.entered_volume_total_ml())
        .bind(valid.feeling_rating)
        .execute(&mut *tx)
        .await
        .map_err(as_api_error)?;
    insert_components(&mut tx, id, &valid).await?;

    tx.commit().await.map_err(as_api_error)?;
    Ok(Written::Done)
}

/// Modifies a drink.
///
/// `merge` receives the stored drink, locked, and answers the drink to write.
/// The components are rewritten only when `replace_components` says the
/// request carried a new composition.
///
/// # Errors
///
/// Answers a validation failure for an invalid drink or an event the profile
/// does not take part in, and fails on any other write error.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    replace_components: bool,
    merge: impl FnOnce(Drink) -> Result<ValidDrink, ApiError>,
) -> Result<Written, ApiError> {
    let mut tx = pool.begin().await?;

    let Some(stored) = fetch(&mut tx, SELECT_DRINK_FOR_UPDATE, id).await? else {
        return Ok(Written::NotFound);
    };
    let valid = merge(stored)?;

    sqlx::query(UPDATE_DRINK)
        .bind(id)
        .bind(valid.event_id)
        .bind(valid.drink.name())
        .bind(valid.ingested_at)
        .bind(seconds(valid.drink.ingestion_duration_seconds())?)
        .bind(seconds(valid.drink.absorption_duration_seconds())?)
        .bind(QuantityUnit::from(valid.drink.unit()).as_sql())
        .bind(valid.drink.entered_volume_total_ml())
        .bind(valid.feeling_rating)
        .execute(&mut *tx)
        .await
        .map_err(as_api_error)?;

    if replace_components {
        sqlx::query(DELETE_COMPONENTS)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(as_api_error)?;
        insert_components(&mut tx, id, &valid).await?;
    }

    tx.commit().await.map_err(as_api_error)?;
    Ok(Written::Done)
}

/// Deletes a drink and its components.
///
/// # Errors
///
/// Fails when the statement cannot be run.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<Written, ApiError> {
    let deleted = sqlx::query(DELETE_DRINK)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(if deleted == 0 {
        Written::NotFound
    } else {
        Written::Done
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_answered_as_a_field_error_is_the_one_the_schema_declares() {
        // `as_api_error` recognises a drink attached to an event its profile
        // does not take part in by the *name* of the foreign key PostgreSQL
        // reports. Renamed on the schema side only, the same request turns
        // from a 400 into a 500. Lexical, like its sibling in
        // `profile::store`; the live check is in `crates/api/tests/database.rs`.
        let declaration =
            format!("constraint {EVENT_PARTICIPANT_KEY} foreign key (event_id, profile_id)");
        let mut declared = 0;
        for migration in db::MIGRATOR.migrations.iter() {
            let words = migration.sql.to_lowercase();
            let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
            declared += words.matches(&declaration).count();
        }
        assert_eq!(
            declared, 1,
            "the migrations do not declare `{declaration}` exactly once"
        );
    }

    #[test]
    fn every_statement_names_its_relations_and_types_with_their_schema() {
        // Class (e) of the migration of #2: `pg_temp` is searched first, so an
        // unqualified name can be shadowed by a temporary table.
        for statement in [
            SELECT_PROFILE_DEFAULTS,
            INSERT_DRINK,
            INSERT_COMPONENT,
            SELECT_DRINK,
            SELECT_DRINK_FOR_UPDATE,
            SELECT_COMPONENTS,
            UPDATE_DRINK,
            DELETE_COMPONENTS,
            DELETE_DRINK,
        ] {
            for keyword in ["FROM ", "INTO ", "UPDATE ", "::"] {
                for (at, _) in statement.match_indices(keyword) {
                    let rest = &statement[at + keyword.len()..];
                    if keyword == "UPDATE " && statement[..at].ends_with("FOR ") {
                        continue;
                    }
                    assert!(
                        rest.starts_with("public.") || rest.starts_with("pg_catalog."),
                        "unqualified name after `{keyword}` in {statement}"
                    );
                }
            }
        }
    }
}
