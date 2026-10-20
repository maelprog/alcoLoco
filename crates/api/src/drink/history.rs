//! `GET /profiles/{id}/drinks` — the history of a profile (SPEC.md §5.3 mode
//! 3, §5.5), read one page at a time.
//!
//! **Which drinks.** Every drink of the profile, whatever the event it was
//! entered from, or none: a drink always belongs to the history of its profile
//! (SPEC.md §4), and the duplication of mode 3 offers *all* of them, not those
//! of the event in progress. Three optional filters narrow it: `from`
//! (inclusive) and `before` (exclusive) on the ingestion time, and `event_id`.
//!
//! **In which order.** Latest ingestion first, the identifier breaking ties —
//! the drink recorded last first. That order is total, which is what lets a
//! page resume exactly after the previous one.
//!
//! **Why the cursor is not an identifier.** An identifier says where a drink
//! was *recorded*, not where it sits in this order: a drink entered
//! retroactively has a recent identifier and an old ingestion time. The cursor
//! is therefore the sort key itself, `(ingested_at, id)` of the last drink
//! served, and a page reads the drinks strictly after it in the order above.
//! It names a position, not a row, so it stays valid when that drink is
//! deleted or modified meanwhile, and a drink recorded meanwhile cannot shift
//! the rows still to come: it lands either before the cursor — already walked,
//! never served twice — or after it, served in its place.

use chrono::SecondsFormat;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::error::FieldError;
use crate::pagination::Cursor;
use crate::timestamp::Timestamp;

use super::validation::earliest_ingested_at;

/// The `?from=&before=&event_id=` filters, exactly as they arrive.
///
/// Read as text and validated by [`HistoryQuery::into_filter`], like
/// [`crate::pagination::PageQuery`]: an unreadable member is a field error
/// inside a problem document, reported with everything else wrong with the
/// request.
#[derive(Debug, Default, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryQuery {
    /// Earliest ingestion time kept, inclusive. Absent: no lower bound.
    #[param(value_type = Option<String>, format = DateTime, example = "2026-08-20T00:00:00Z")]
    pub from: Option<String>,
    /// Ingestion time from which drinks are left out, exclusive. Absent: no
    /// upper bound.
    #[param(value_type = Option<String>, format = DateTime, example = "2026-08-21T00:00:00Z")]
    pub before: Option<String>,
    /// Event whose drinks are kept. Absent: every drink, event or not.
    #[param(value_type = Option<Uuid>)]
    pub event_id: Option<String>,
}

/// Validated filters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    /// Inclusive lower bound on `ingested_at`.
    pub from: Option<Timestamp>,
    /// Exclusive upper bound on `ingested_at`.
    pub before: Option<Timestamp>,
    /// Event to keep the drinks of.
    pub event_id: Option<Uuid>,
}

/// The trimmed value, or `None` when it is absent or blank.
fn blank_to_none(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Reads an instant, clamped to the earliest one a `timestamptz` holds.
///
/// `chrono` reads instants PostgreSQL refuses, with a 500. Clamping filters
/// exactly what was asked: no drink is ingested earlier than
/// [`earliest_ingested_at`], so a bound below it keeps — or, for `before`,
/// leaves out — the same drinks as that bound.
fn instant(text: &str) -> Option<Timestamp> {
    text.parse::<Timestamp>()
        .ok()
        .map(|at| at.max(earliest_ingested_at()))
}

impl HistoryQuery {
    /// Validates the filters, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns the field errors of each member that could not be used.
    pub fn into_filter(self) -> Result<HistoryFilter, Vec<FieldError>> {
        let mut errors = Vec::new();
        let mut read_instant = |field: &str, value: Option<&str>| {
            let text = blank_to_none(value)?;
            let at = instant(text);
            if at.is_none() {
                errors.push(FieldError::new(
                    field,
                    "must be an RFC 3339 date-time, e.g. 2026-08-20T21:04:05Z",
                ));
            }
            at
        };
        let from = read_instant("from", self.from.as_deref());
        let before = read_instant("before", self.before.as_deref());

        if let (Some(from), Some(before)) = (from, before)
            && before <= from
        {
            errors.push(FieldError::new("before", "must be later than `from`"));
        }

        let event_id = blank_to_none(self.event_id.as_deref()).and_then(|text| {
            let id = text.parse::<Uuid>().ok();
            if id.is_none() {
                errors.push(FieldError::new("event_id", "must be a UUID"));
            }
            id
        });

        if errors.is_empty() {
            Ok(HistoryFilter {
                from,
                before,
                event_id,
            })
        } else {
            Err(errors)
        }
    }
}

/// Position of a drink in the history: its sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryCursor {
    /// Ingestion time of the last drink served.
    pub ingested_at: Timestamp,
    /// Identifier of the last drink served, which breaks ties.
    pub id: Uuid,
}

/// Separates the two members of a rendered cursor. Neither an RFC 3339
/// instant in UTC nor a hyphenated UUID contains it.
const CURSOR_SEPARATOR: char = '_';

impl Cursor for HistoryCursor {
    fn parse(text: &str) -> Option<Self> {
        let (at, id) = text.split_once(CURSOR_SEPARATOR)?;
        let ingested_at = at.parse::<Timestamp>().ok()?;
        // Rendered by the API, a cursor never lies outside what the database
        // holds; one that does was forged, and would be a 500.
        if ingested_at < earliest_ingested_at() {
            return None;
        }
        Some(Self {
            ingested_at,
            id: id.parse().ok()?,
        })
    }

    fn render(&self) -> String {
        // Microseconds: the precision of a `timestamptz`, so that the instant
        // read back is the instant stored, to the bit.
        format!(
            "{}{CURSOR_SEPARATOR}{}",
            self.ingested_at
                .to_rfc3339_opts(SecondsFormat::Micros, true),
            self.id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(from: Option<&str>, before: Option<&str>, event_id: Option<&str>) -> HistoryQuery {
        HistoryQuery {
            from: from.map(str::to_owned),
            before: before.map(str::to_owned),
            event_id: event_id.map(str::to_owned),
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a real instant")
    }

    fn fields(errors: &[FieldError]) -> Vec<&str> {
        errors.iter().map(|error| error.field.as_str()).collect()
    }

    #[test]
    fn no_filter_keeps_every_drink() {
        for blank in [None, Some(""), Some("  ")] {
            assert_eq!(
                query(blank, blank, blank).into_filter(),
                Ok(HistoryFilter::default()),
                "{blank:?}"
            );
        }
    }

    #[test]
    fn the_filters_are_read_as_given() {
        let event = crate::id::new_id();
        let filter = query(
            Some("2026-08-20T00:00:00Z"),
            Some(" 2026-08-21T00:00:00Z "),
            Some(&event.to_string()),
        )
        .into_filter()
        .expect("must validate");
        assert_eq!(
            filter,
            HistoryFilter {
                from: Some(at("2026-08-20T00:00:00Z")),
                before: Some(at("2026-08-21T00:00:00Z")),
                event_id: Some(event),
            }
        );
    }

    #[test]
    fn an_offset_is_read_as_the_instant_it_names() {
        let filter = query(Some("2026-08-20T02:00:00+02:00"), None, None)
            .into_filter()
            .expect("must validate");
        assert_eq!(filter.from, Some(at("2026-08-20T00:00:00Z")));
    }

    #[test]
    fn every_unreadable_member_is_reported_in_one_answer() {
        let errors = query(
            Some("yesterday"),
            Some("2026-13-01T00:00:00Z"),
            Some("soirée"),
        )
        .into_filter()
        .expect_err("must fail");
        assert_eq!(fields(&errors), ["from", "before", "event_id"]);
    }

    #[test]
    fn an_empty_or_reversed_range_is_a_field_error_on_its_end() {
        for (from, before) in [
            ("2026-08-21T00:00:00Z", "2026-08-21T00:00:00Z"),
            ("2026-08-21T00:00:00Z", "2026-08-20T00:00:00Z"),
        ] {
            let errors = query(Some(from), Some(before), None)
                .into_filter()
                .expect_err("must fail");
            assert_eq!(fields(&errors), ["before"], "{from} .. {before}");
        }
        let one_microsecond = query(
            Some("2026-08-21T00:00:00Z"),
            Some("2026-08-21T00:00:00.000001Z"),
            None,
        )
        .into_filter();
        assert!(one_microsecond.is_ok(), "{one_microsecond:?}");
    }

    #[test]
    fn a_bound_earlier_than_the_database_holds_is_clamped_to_its_earliest_instant() {
        let filter = query(Some("-5000-01-01T00:00:00Z"), None, None)
            .into_filter()
            .expect("must validate");
        assert_eq!(filter.from, Some(earliest_ingested_at()));
        let filter = query(None, Some("-5000-01-01T00:00:00Z"), None)
            .into_filter()
            .expect("must validate");
        assert_eq!(filter.before, Some(earliest_ingested_at()));
    }

    #[test]
    fn a_cursor_reads_back_the_position_it_renders_to_the_microsecond() {
        let cursor = HistoryCursor {
            ingested_at: at("2026-08-20T21:04:05.123456Z"),
            id: crate::id::new_id(),
        };
        let text = cursor.render();
        assert_eq!(
            text,
            format!("2026-08-20T21:04:05.123456Z_{}", cursor.id),
            "the rendering is what clients are handed"
        );
        assert_eq!(HistoryCursor::parse(&text), Some(cursor));

        let whole_second = HistoryCursor {
            ingested_at: at("2026-08-20T21:04:05Z"),
            id: cursor.id,
        };
        assert_eq!(
            HistoryCursor::parse(&whole_second.render()),
            Some(whole_second)
        );
    }

    #[test]
    fn a_text_that_is_not_a_rendered_cursor_is_refused() {
        let id = crate::id::new_id();
        for text in [
            id.to_string(),
            "2026-08-20T21:04:05Z".to_owned(),
            format!("yesterday_{id}"),
            "2026-08-20T21:04:05Z_nope".to_owned(),
            format!("-5000-01-01T00:00:00Z_{id}"),
        ] {
            assert_eq!(HistoryCursor::parse(&text), None, "{text}");
        }
    }

    #[test]
    fn the_earliest_instant_the_database_holds_is_a_legal_cursor() {
        let cursor = HistoryCursor {
            ingested_at: earliest_ingested_at(),
            id: Uuid::nil(),
        };
        assert_eq!(HistoryCursor::parse(&cursor.render()), Some(cursor));
    }
}
