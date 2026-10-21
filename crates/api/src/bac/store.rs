//! Every statement the curve endpoints run.
//!
//! The rules of `profile::store` hold here too: every statement is a named
//! `&'static str` and values reach PostgreSQL through `bind` only; relations,
//! types and functions are schema qualified.
//!
//! Each drink is read with the settings version in force at its ingestion
//! time, through `profile_settings_at` — the resolution every computation goes
//! through (`profile::store::settings_at`) — joined `LEFT JOIN LATERAL … ON
//! true` so that a drink is never dropped with missing settings. One answer
//! reads one snapshot: the profile or the event, its participants, the drinks
//! and their components as they all stood at the same instant.

use std::collections::HashMap;

use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::drink::Component;
use crate::drink::store::drink_from_row;
use crate::error::ApiError;
use crate::profile::store::settings_from_row;
use crate::timestamp::Timestamp;

use super::query::Grid;
use super::series::StoredDrink;

/// Makes the reads of one answer see one snapshot.
const SNAPSHOT_READ: &str = "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY";

/// Whether a profile exists, so that an unknown one is told from one that
/// never drank.
const SELECT_PROFILE_EXISTS: &str = "SELECT 1 FROM public.profile WHERE id = $1";

/// The window of an event.
const SELECT_EVENT: &str = "SELECT starts_at, ends_at FROM public.event WHERE id = $1";

/// The participants of an event, in the order they were added, ties broken by
/// profile identifier — the order of the event endpoints.
const SELECT_PARTICIPANTS: &str = "
    SELECT ep.profile_id, p.display_name
    FROM public.event_participant AS ep
    JOIN public.profile AS p ON p.id = ep.profile_id
    WHERE ep.event_id = $1
    ORDER BY ep.joined_at, ep.profile_id
";

/// Every drink of the profiles `$1` ingested before `$2`, each with the
/// settings in force at its ingestion time. Every drink of the history counts,
/// whatever the event it was entered from: the curve is the body's, not the
/// event's (SPEC.md §4).
const SELECT_DRINKS: &str = "
    SELECT d.id, d.profile_id, d.event_id, d.name, d.ingested_at,
           d.ingestion_duration_seconds, d.absorption_duration_seconds,
           d.quantity_unit::pg_catalog.text AS quantity_unit,
           d.volume_total_ml, d.feeling_rating, d.created_at, d.updated_at,
           s.valid_from, s.weight_kg, s.height_cm, s.sex::pg_catalog.text AS sex, s.birth_date
    FROM public.drink AS d
    LEFT JOIN LATERAL public.profile_settings_at(d.profile_id, d.ingested_at) AS s ON true
    WHERE d.profile_id = ANY($1::pg_catalog.uuid[])
      AND d.ingested_at < $2
    ORDER BY d.profile_id, d.ingested_at, d.id
";

/// The components of the drinks [`SELECT_DRINKS`] reads, each drink's in the
/// order they were added.
const SELECT_COMPONENTS: &str = "
    SELECT c.drink_id, c.name, c.abv_percent, c.quantity
    FROM public.drink_component AS c
    JOIN public.drink AS d ON d.id = c.drink_id
    WHERE d.profile_id = ANY($1::pg_catalog.uuid[])
      AND d.ingested_at < $2
    ORDER BY c.drink_id, c.sort_order
";

/// A participant of an event and its drinks.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticipantDrinks {
    /// Identifier of the profile.
    pub profile_id: Uuid,
    /// Name of the profile.
    pub display_name: String,
    /// Its drinks, oldest ingestion first.
    pub drinks: Vec<StoredDrink>,
}

/// An event as its curves need it.
#[derive(Debug, Clone, PartialEq)]
pub struct EventDrinks {
    /// The grid the caller built from the window of the event.
    pub grid: Grid,
    /// The participants, in the order they were added.
    pub participants: Vec<ParticipantDrinks>,
}

/// The drinks of `profiles` ingested before `before`, grouped by profile.
async fn drinks_of(
    tx: &mut Transaction<'_, Postgres>,
    profiles: &[Uuid],
    before: Timestamp,
) -> Result<HashMap<Uuid, Vec<StoredDrink>>, ApiError> {
    let mut components: HashMap<Uuid, Vec<Component>> = HashMap::new();
    for row in sqlx::query(SELECT_COMPONENTS)
        .bind(profiles)
        .bind(before)
        .fetch_all(&mut **tx)
        .await?
    {
        components
            .entry(row.try_get("drink_id")?)
            .or_default()
            .push(Component {
                name: row.try_get("name")?,
                abv_percent: row.try_get("abv_percent")?,
                quantity: row.try_get("quantity")?,
            });
    }

    let mut drinks: HashMap<Uuid, Vec<StoredDrink>> = HashMap::new();
    for row in sqlx::query(SELECT_DRINKS)
        .bind(profiles)
        .bind(before)
        .fetch_all(&mut **tx)
        .await?
    {
        let id: Uuid = row.try_get("id")?;
        let drink = drink_from_row(&row, components.remove(&id).unwrap_or_default())?;
        let settings = settings_from_row(&row)?;
        drinks
            .entry(drink.profile_id)
            .or_default()
            .push(StoredDrink { drink, settings });
    }
    Ok(drinks)
}

/// The drinks of a profile ingested before `before`, oldest first, or nothing
/// when no profile carries that identifier.
///
/// # Errors
///
/// Fails when a query cannot be run or a row cannot be read.
pub async fn profile_drinks(
    pool: &PgPool,
    profile_id: Uuid,
    before: Timestamp,
) -> Result<Option<Vec<StoredDrink>>, ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query(SNAPSHOT_READ).execute(&mut *tx).await?;
    if sqlx::query(SELECT_PROFILE_EXISTS)
        .bind(profile_id)
        .fetch_optional(&mut *tx)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let mut drinks = drinks_of(&mut tx, &[profile_id], before).await?;
    tx.commit().await?;
    Ok(Some(drinks.remove(&profile_id).unwrap_or_default()))
}

/// The participants of an event and their drinks, or nothing when no event
/// carries that identifier.
///
/// `window` receives the start and the end of the event and answers the grid
/// to sample, from which `before` is drawn: the drinks ingested from
/// `before(grid)` on cannot change a value of the answer.
///
/// # Errors
///
/// What `window` answers, and failures to run a query or read a row.
pub async fn event_drinks(
    pool: &PgPool,
    event_id: Uuid,
    window: impl FnOnce(Timestamp, Option<Timestamp>) -> Result<Grid, ApiError>,
    before: impl FnOnce(Grid) -> Result<Timestamp, ApiError>,
) -> Result<Option<EventDrinks>, ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query(SNAPSHOT_READ).execute(&mut *tx).await?;
    let Some(event) = sqlx::query(SELECT_EVENT)
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(None);
    };
    let grid = window(event.try_get("starts_at")?, event.try_get("ends_at")?)?;

    let rows = sqlx::query(SELECT_PARTICIPANTS)
        .bind(event_id)
        .fetch_all(&mut *tx)
        .await?;
    let ids: Vec<Uuid> = rows
        .iter()
        .map(|row| row.try_get("profile_id"))
        .collect::<Result<_, _>>()?;
    let mut drinks = drinks_of(&mut tx, &ids, before(grid)?).await?;
    tx.commit().await?;

    let participants = rows
        .iter()
        .zip(ids)
        .map(|(row, profile_id)| {
            Ok(ParticipantDrinks {
                profile_id,
                display_name: row.try_get("display_name")?,
                drinks: drinks.remove(&profile_id).unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Some(EventDrinks { grid, participants }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_statement_names_its_relations_types_and_functions_with_their_schema() {
        // Class (e) of the migration of #2: `pg_temp` is searched first, so an
        // unqualified relation or function can be shadowed by a temporary one.
        for statement in [
            SNAPSHOT_READ,
            SELECT_PROFILE_EXISTS,
            SELECT_EVENT,
            SELECT_PARTICIPANTS,
            SELECT_DRINKS,
            SELECT_COMPONENTS,
        ] {
            for keyword in ["FROM ", "JOIN ", "LATERAL ", "::"] {
                for (at, _) in statement.match_indices(keyword) {
                    let rest = &statement[at + keyword.len()..];
                    let rest = rest.strip_prefix("LATERAL ").unwrap_or(rest);
                    assert!(
                        rest.starts_with("public.") || rest.starts_with("pg_catalog."),
                        "unqualified name after `{keyword}` in {statement}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_settings_of_a_drink_are_those_of_its_ingestion_time() {
        // The resolution of SPEC.md §5.6 is the database function, called at
        // the ingestion time of each drink, never at the instant of the
        // request.
        assert!(
            SELECT_DRINKS.contains("public.profile_settings_at(d.profile_id, d.ingested_at)"),
            "{SELECT_DRINKS}"
        );
        assert!(
            SELECT_DRINKS.contains("LEFT JOIN LATERAL"),
            "{SELECT_DRINKS}"
        );
    }
}
