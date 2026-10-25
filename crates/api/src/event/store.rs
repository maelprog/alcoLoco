//! Every statement the event endpoints run.
//!
//! The rules of `profile::store` hold here too: every statement is a named
//! `&'static str` and values reach PostgreSQL through `bind` only; relations
//! and types are schema qualified.
//!
//! The existence of an event or a profile a write is about to reference is
//! read under `FOR KEY SHARE`, so that it cannot be deleted before the write
//! commits, and so that an unknown one is told apart by the statement that
//! looked for it rather than by the name of the foreign key it would violate.

use std::collections::HashMap;

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};
use crate::pagination::{Page, PageRequest};

use super::model::{Event, Participant};
use super::validation::{ValidCreation, ValidEvent};

/// Makes the reads of one answer see one snapshot: an event and its
/// participants as they stood at the same instant.
const SNAPSHOT_READ: &str = "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY";

/// One event, without its participants.
const SELECT_EVENT: &str = "
    SELECT id, name, starts_at, ends_at, created_at, updated_at
    FROM public.event
    WHERE id = $1
";

/// One page of events in identifier order, that is creation order, resumed
/// strictly after the cursor `$1`, which may be null.
const SELECT_EVENT_PAGE: &str = "
    SELECT id, name, starts_at, ends_at, created_at, updated_at
    FROM public.event
    WHERE $1::pg_catalog.uuid IS NULL OR id > $1::pg_catalog.uuid
    ORDER BY id
    LIMIT $2
";

/// The participants of several events, each event's in the order they were
/// added, ties broken by profile identifier.
const SELECT_PARTICIPANTS_OF: &str = "
    SELECT ep.event_id, ep.profile_id, p.display_name, ep.joined_at
    FROM public.event_participant AS ep
    JOIN public.profile AS p ON p.id = ep.profile_id
    WHERE ep.event_id = ANY($1::pg_catalog.uuid[])
    ORDER BY ep.event_id, ep.joined_at, ep.profile_id
";

/// Those of the profiles `$1` that exist, kept from deletion until the
/// transaction ends.
const SELECT_EXISTING_PROFILES: &str = "
    SELECT id
    FROM public.profile
    WHERE id = ANY($1::pg_catalog.uuid[])
    FOR KEY SHARE
";

/// The event row.
const INSERT_EVENT: &str = "
    INSERT INTO public.event (id, name, starts_at, ends_at)
    VALUES ($1, $2, $3, $4)
";

/// The first participants of a new event, in one statement.
const INSERT_PARTICIPANTS: &str = "
    INSERT INTO public.event_participant (event_id, profile_id)
    SELECT $1, profile_id
    FROM pg_catalog.unnest($2::pg_catalog.uuid[]) AS profile_id
";

/// The three members of an event; its participants are left alone.
const UPDATE_EVENT: &str = "
    UPDATE public.event
    SET name = $2,
        starts_at = $3,
        ends_at = $4
    WHERE id = $1
";

/// Whether an event exists, kept from deletion until the transaction ends.
const LOCK_EVENT: &str = "SELECT 1 FROM public.event WHERE id = $1 FOR KEY SHARE";

/// Whether a profile exists, kept from deletion until the transaction ends.
const LOCK_PROFILE: &str = "SELECT 1 FROM public.profile WHERE id = $1 FOR KEY SHARE";

/// One participant; adding a profile already there changes nothing.
const INSERT_PARTICIPANT: &str = "
    INSERT INTO public.event_participant (event_id, profile_id)
    VALUES ($1, $2)
    ON CONFLICT (event_id, profile_id) DO NOTHING
";

/// One participant. The drinks it recorded from the event stay in its
/// history, detached from the event by the foreign key of `drink`
/// (`ON DELETE SET NULL (event_id)`, migration of #2).
const DELETE_PARTICIPANT: &str = "
    DELETE FROM public.event_participant
    WHERE event_id = $1 AND profile_id = $2
";

/// Whether an event exists, to tell an unknown event from a profile that
/// does not take part in it.
const SELECT_EVENT_EXISTS: &str = "SELECT 1 FROM public.event WHERE id = $1";

/// An event. Its participants go with it (`event_participant`, `ON DELETE
/// CASCADE`), and the drinks entered from it stay in the history of their
/// profile, detached from it by the foreign key of `drink` (`ON DELETE SET
/// NULL (event_id)`), both from the migration of #2.
const DELETE_EVENT: &str = "DELETE FROM public.event WHERE id = $1";

/// Reads a row of [`SELECT_EVENT`], participants aside.
fn event_from_row(row: &PgRow, participants: Vec<Participant>) -> Result<Event, ApiError> {
    Ok(Event {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        starts_at: row.try_get("starts_at")?,
        ends_at: row.try_get("ends_at")?,
        participants,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// The participants of the events `ids`, grouped by event.
async fn participants_of(
    tx: &mut Transaction<'_, Postgres>,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Participant>>, ApiError> {
    let mut participants: HashMap<Uuid, Vec<Participant>> = HashMap::with_capacity(ids.len());
    for row in sqlx::query(SELECT_PARTICIPANTS_OF)
        .bind(ids)
        .fetch_all(&mut **tx)
        .await?
    {
        participants
            .entry(row.try_get("event_id")?)
            .or_default()
            .push(Participant {
                profile_id: row.try_get("profile_id")?,
                display_name: row.try_get("display_name")?,
                joined_at: row.try_get("joined_at")?,
            });
    }
    Ok(participants)
}

/// Reads one event with its participants, or nothing when no event carries
/// that identifier.
///
/// # Errors
///
/// Fails when a query cannot be run or a row cannot be read.
pub async fn read(pool: &PgPool, id: Uuid) -> Result<Option<Event>, ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query(SNAPSHOT_READ).execute(&mut *tx).await?;
    let Some(row) = sqlx::query(SELECT_EVENT)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(None);
    };
    let mut participants = participants_of(&mut tx, &[id]).await?;
    tx.commit().await?;
    event_from_row(&row, participants.remove(&id).unwrap_or_default()).map(Some)
}

/// Reads one page of events, in creation order, each with its participants.
///
/// # Errors
///
/// Fails when a query cannot be run or a row cannot be read.
pub async fn list(pool: &PgPool, request: &PageRequest) -> Result<Page<Event>, ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query(SNAPSHOT_READ).execute(&mut *tx).await?;
    let rows = sqlx::query(SELECT_EVENT_PAGE)
        .bind(request.cursor().copied())
        .bind(request.fetch_limit())
        .fetch_all(&mut *tx)
        .await?;
    let ids: Vec<Uuid> = rows
        .iter()
        .map(|row| row.try_get("id"))
        .collect::<Result<_, _>>()?;
    let mut participants = participants_of(&mut tx, &ids).await?;
    tx.commit().await?;

    let events = rows
        .iter()
        .zip(ids)
        .map(|(row, id)| event_from_row(row, participants.remove(&id).unwrap_or_default()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page::from_rows(events, request, |event| event.id))
}

/// Creates an event with its first participants.
///
/// # Errors
///
/// Answers a validation failure naming each participant that is no profile,
/// and fails on any other write error.
pub async fn create(pool: &PgPool, id: Uuid, creation: &ValidCreation) -> Result<(), ApiError> {
    let mut tx = pool.begin().await?;

    let existing: Vec<Uuid> = sqlx::query_scalar(SELECT_EXISTING_PROFILES)
        .bind(&creation.participant_ids)
        .fetch_all(&mut *tx)
        .await?;
    let unknown: Vec<FieldError> = creation
        .participant_ids
        .iter()
        .filter(|profile| !existing.contains(profile))
        .map(|profile| {
            FieldError::new(
                "participant_ids",
                format!("no profile with identifier {profile}"),
            )
        })
        .collect();
    if !unknown.is_empty() {
        return Err(ApiError::validation(unknown));
    }

    let event = &creation.event;
    sqlx::query(INSERT_EVENT)
        .bind(id)
        .bind(&event.name)
        .bind(event.starts_at)
        .bind(event.ends_at)
        .execute(&mut *tx)
        .await?;
    sqlx::query(INSERT_PARTICIPANTS)
        .bind(id)
        .bind(&creation.participant_ids)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// What a write found, so that the handler can answer the right status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// No event carries the identifier addressed.
    NoEvent,
    /// No profile carries the identifier addressed.
    NoProfile,
    /// The profile addressed does not take part in the event.
    NotParticipating,
    /// The write was made, or had nothing left to do.
    Done,
}

/// Rewrites the name and the times of an event.
///
/// # Errors
///
/// Fails when the statement cannot be run.
pub async fn update(pool: &PgPool, id: Uuid, event: &ValidEvent) -> Result<Written, ApiError> {
    let updated = sqlx::query(UPDATE_EVENT)
        .bind(id)
        .bind(&event.name)
        .bind(event.starts_at)
        .bind(event.ends_at)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(if updated == 0 {
        Written::NoEvent
    } else {
        Written::Done
    })
}

/// Adds a profile to an event; adding one already there changes nothing.
///
/// # Errors
///
/// Fails when a statement cannot be run.
pub async fn add_participant(
    pool: &PgPool,
    event_id: Uuid,
    profile_id: Uuid,
) -> Result<Written, ApiError> {
    let mut tx = pool.begin().await?;
    for (statement, id, missing) in [
        (LOCK_EVENT, event_id, Written::NoEvent),
        (LOCK_PROFILE, profile_id, Written::NoProfile),
    ] {
        if sqlx::query(statement)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .is_none()
        {
            return Ok(missing);
        }
    }
    sqlx::query(INSERT_PARTICIPANT)
        .bind(event_id)
        .bind(profile_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Written::Done)
}

/// Removes a profile from an event. Its drinks stay in its history.
///
/// # Errors
///
/// Fails when a statement cannot be run.
pub async fn remove_participant(
    pool: &PgPool,
    event_id: Uuid,
    profile_id: Uuid,
) -> Result<Written, ApiError> {
    let deleted = sqlx::query(DELETE_PARTICIPANT)
        .bind(event_id)
        .bind(profile_id)
        .execute(pool)
        .await?
        .rows_affected();
    if deleted > 0 {
        return Ok(Written::Done);
    }
    let event_exists = sqlx::query(SELECT_EVENT_EXISTS)
        .bind(event_id)
        .fetch_optional(pool)
        .await?
        .is_some();
    Ok(if event_exists {
        Written::NotParticipating
    } else {
        Written::NoEvent
    })
}

/// Deletes an event and its participants. The drinks entered from it stay
/// in the history of their profile, detached from it.
///
/// # Errors
///
/// Fails when the statement cannot be run.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<Written, ApiError> {
    let deleted = sqlx::query(DELETE_EVENT)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(if deleted == 0 {
        Written::NoEvent
    } else {
        Written::Done
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_statement_names_its_relations_types_and_functions_with_their_schema() {
        // Class (e) of the migration of #2: `pg_temp` is searched first, so an
        // unqualified relation can be shadowed by a temporary table.
        for statement in [
            SNAPSHOT_READ,
            SELECT_EVENT,
            SELECT_EVENT_PAGE,
            SELECT_PARTICIPANTS_OF,
            SELECT_EXISTING_PROFILES,
            INSERT_EVENT,
            INSERT_PARTICIPANTS,
            UPDATE_EVENT,
            LOCK_EVENT,
            LOCK_PROFILE,
            INSERT_PARTICIPANT,
            DELETE_PARTICIPANT,
            SELECT_EVENT_EXISTS,
            DELETE_EVENT,
        ] {
            for keyword in ["FROM ", "INTO ", "UPDATE ", "JOIN ", "::"] {
                for (at, _) in statement.match_indices(keyword) {
                    let rest = &statement[at + keyword.len()..];
                    assert!(
                        rest.starts_with("public.") || rest.starts_with("pg_catalog."),
                        "unqualified name after `{keyword}` in {statement}"
                    );
                }
            }
        }
    }
}
