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

/// Reads an instant, rounded up to the microsecond and clamped to the
/// earliest one a `timestamptz` holds.
///
/// Both keep the filter exact. A `timestamptz` holds microseconds, and a finer
/// bound would reach PostgreSQL truncated: half a microsecond after a drink,
/// `from` would keep it and `before` leave it out. Rounded up, the bound keeps
/// — under `from` — or leaves out — under `before` — exactly the stored
/// instants it did. `chrono` also reads instants PostgreSQL refuses, with a
/// 500; no drink is ingested earlier than [`earliest_ingested_at`], so a
/// bound below it filters the same drinks as that bound.
fn instant(text: &str) -> Option<Timestamp> {
    let at = text.parse::<Timestamp>().ok()?;
    let below_a_microsecond = i64::from(at.timestamp_subsec_nanos() % 1_000);
    let at = if below_a_microsecond == 0 {
        at
    } else {
        at.checked_add_signed(chrono::TimeDelta::nanoseconds(1_000 - below_a_microsecond))?
    };
    Some(at.max(earliest_ingested_at()))
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

/// Separates the two members of a rendered cursor. Neither a signed integer
/// nor a hyphenated UUID contains it.
const CURSOR_SEPARATOR: char = '_';

/// A cursor is rendered `<microseconds since the Unix epoch>_<identifier>`:
/// digits, `-`, `_` and hexadecimal only, all of them unreserved in RFC 3986,
/// so that a client may put it in a URL as it received it. An RFC 3339 instant
/// would not do: from the year 10000 on it starts with `+`, which a query
/// string reads as a space. Microseconds are the precision of a
/// `timestamptz`, so the instant read back is the instant stored, to the bit.
impl Cursor for HistoryCursor {
    fn parse(text: &str) -> Option<Self> {
        let (micros, id) = text.split_once(CURSOR_SEPARATOR)?;
        let ingested_at = Timestamp::from_timestamp_micros(micros.parse().ok()?)?;
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
        format!(
            "{}{CURSOR_SEPARATOR}{}",
            self.ingested_at.timestamp_micros(),
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
    fn a_bound_finer_than_the_microsecond_is_rounded_up_to_the_next_one() {
        // The database stores microseconds and the bound reaches it truncated
        // to them. Rounded up instead, `from` still keeps exactly the drinks
        // at or after it, and `before` exactly those before it.
        let filter = query(
            Some("2026-08-21T21:00:00.0000005Z"),
            Some("2026-08-21T22:00:00.0000005Z"),
            None,
        )
        .into_filter()
        .expect("must validate");
        assert_eq!(filter.from, Some(at("2026-08-21T21:00:00.000001Z")));
        assert_eq!(filter.before, Some(at("2026-08-21T22:00:00.000001Z")));

        let whole = query(Some("2026-08-21T21:00:00.000001Z"), None, None)
            .into_filter()
            .expect("must validate");
        assert_eq!(whole.from, Some(at("2026-08-21T21:00:00.000001Z")));
    }

    #[test]
    fn a_range_holding_no_microsecond_is_a_field_error() {
        let errors = query(
            Some("2026-08-21T21:00:00.0000001Z"),
            Some("2026-08-21T21:00:00.0000002Z"),
            None,
        )
        .into_filter()
        .expect_err("must fail");
        assert_eq!(fields(&errors), ["before"]);
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
            format!("1787259845123456_{}", cursor.id),
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
    fn a_cursor_is_safe_in_a_query_string_whatever_the_year() {
        // A client is handed the cursor and may put it in a URL as it is. An
        // RFC 3339 instant from the year 10000 on starts with `+`, which a
        // query string reads as a space. Only the unreserved characters of
        // RFC 3986 are safe unencoded.
        for ingested_at in [
            earliest_ingested_at(),
            at("1969-12-31T23:59:59.999999Z"),
            at("2026-08-20T21:04:05.123456Z"),
            at("+10000-01-01T00:00:00Z"),
            at("+262142-12-31T23:59:59.999999Z"),
        ] {
            let cursor = HistoryCursor {
                ingested_at,
                id: crate::id::new_id(),
            };
            let text = cursor.render();
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)),
                "{ingested_at}: {text}"
            );
            assert_eq!(HistoryCursor::parse(&text), Some(cursor), "{text}");
        }
    }

    #[test]
    fn a_text_that_is_not_a_rendered_cursor_is_refused() {
        let id = crate::id::new_id();
        let too_early = earliest_ingested_at().timestamp_micros() - 1;
        for text in [
            id.to_string(),
            "1787259845123456".to_owned(),
            format!("yesterday_{id}"),
            format!("2026-08-20T21:04:05Z_{id}"),
            "1787259845123456_nope".to_owned(),
            format!("{too_early}_{id}"),
            format!("{}_{id}", i64::MAX),
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
