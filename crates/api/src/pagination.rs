//! Cursor pagination, shared by every collection the API exposes.
//!
//! The contract is fixed here once: a request carries `?limit=&cursor=`, an
//! answer carries `{ "items": [...], "next_cursor": <string|null> }`. `limit`
//! defaults to [`DEFAULT_LIMIT`] and is capped at [`MAX_LIMIT`].
//!
//! The cursor is opaque to a client: it passes back the `next_cursor` it was
//! given, and nothing else. On the API side it is the sort key of the last
//! row of the page, rendered by a [`Cursor`]. A collection in identifier order
//! — profiles, library — uses the identifier alone: identifiers are UUID v7
//! (see [`crate::id`]), so they sort in creation order, and
//! `WHERE id > $cursor ORDER BY id` resumes exactly where the previous page
//! stopped. A collection in another order carries that order's key, made
//! total by the identifier (see `crate::drink::history`). An `OFFSET` would
//! not do: a row inserted meanwhile shifts every subsequent offset and makes a
//! row appear twice or not at all.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};

/// Page size used when the request does not ask for one.
pub const DEFAULT_LIMIT: u16 = 50;

/// Largest page size the API will serve.
pub const MAX_LIMIT: u16 = 200;

/// The `?limit=&cursor=` pair, exactly as it arrives.
///
/// Both members are read as text and validated by [`PageQuery::into_request`],
/// so that `?limit=abc` and `?limit=1000` are reported the same way — as a field
/// error inside a problem document — instead of one being rejected by the
/// deserialiser before the handler is ever reached. The document publishes
/// `limit` as the integer it has to be; the bounds written there are tied to
/// [`DEFAULT_LIMIT`] and [`MAX_LIMIT`] by a test of `crate::openapi`, since the
/// derive only takes literals.
#[derive(Debug, Default, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// Number of items asked for.
    #[param(value_type = Option<u16>, minimum = 1, maximum = 200, default = 50)]
    pub limit: Option<String>,
    /// The `next_cursor` of the previous page, passed back as it was received.
    pub cursor: Option<String>,
}

/// The sort key a page resumes after, and its rendering as `next_cursor`.
pub trait Cursor: Sized {
    /// Reads back a cursor produced by [`Cursor::render`], or `None` when the
    /// text is not one.
    fn parse(text: &str) -> Option<Self>;

    /// The text a client passes back to obtain the next page.
    fn render(&self) -> String;
}

/// The cursor of a collection in identifier order.
impl Cursor for Uuid {
    fn parse(text: &str) -> Option<Self> {
        text.parse().ok()
    }

    fn render(&self) -> String {
        self.to_string()
    }
}

/// A validated page request.
///
/// Its members are private so that every request holds a page size within
/// `1..=MAX_LIMIT`: a request of zero rows would answer an empty page with no
/// cursor, which reads as "collection exhausted" while rows remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest<C = Uuid> {
    limit: u16,
    cursor: Option<C>,
}

impl<C> PageRequest<C> {
    /// A request for `limit` rows after `cursor`, or `None` when `limit` is
    /// outside `1..=MAX_LIMIT`.
    #[must_use]
    pub fn new(limit: u16, cursor: Option<C>) -> Option<Self> {
        (1..=MAX_LIMIT)
            .contains(&limit)
            .then_some(Self { limit, cursor })
    }

    /// Number of items to return, within `1..=MAX_LIMIT`.
    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }

    /// Exclusive bound, in the collection's order, on the rows to return.
    #[must_use]
    pub const fn cursor(&self) -> Option<&C> {
        self.cursor.as_ref()
    }

    /// How many rows to ask the database for: one more than the page size, so
    /// that the presence of a next page is known without a second query.
    #[must_use]
    pub const fn fetch_limit(&self) -> i64 {
        self.limit as i64 + 1
    }
}

impl PageQuery {
    /// Validates the pair, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that could not be used.
    pub fn into_request<C: Cursor>(self) -> Result<PageRequest<C>, ApiError> {
        self.into_parts().map_err(ApiError::validation)
    }

    /// [`PageQuery::into_request`], with the field errors left unwrapped so
    /// that a handler can report them together with those of its own filters.
    ///
    /// # Errors
    ///
    /// Returns the field errors of each member that could not be used.
    pub fn into_parts<C: Cursor>(self) -> Result<PageRequest<C>, Vec<FieldError>> {
        let mut errors = Vec::new();

        let limit = match self
            .limit
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => Some(DEFAULT_LIMIT),
            Some(text) => text
                .parse::<u16>()
                .ok()
                .filter(|value| (1..=MAX_LIMIT).contains(value)),
        };
        if limit.is_none() {
            errors.push(FieldError::new(
                "limit",
                format!("must be a whole number between 1 and {MAX_LIMIT}"),
            ));
        }

        let cursor = match self
            .cursor
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(text) => {
                let cursor = C::parse(text);
                if cursor.is_none() {
                    errors.push(FieldError::new(
                        "cursor",
                        "must be the `next_cursor` of a previous page",
                    ));
                }
                cursor
            }
        };

        match limit.and_then(|limit| PageRequest::new(limit, cursor)) {
            Some(request) if errors.is_empty() => Ok(request),
            _ => Err(errors),
        }
    }
}

/// One page of a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    /// The items of this page, in the order of the collection.
    pub items: Vec<T>,
    /// Cursor to pass back to obtain the next page, `null` on the last one.
    /// Always present.
    #[schema(required = true)]
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// The last page of a collection.
    #[must_use]
    pub const fn last(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
        }
    }

    /// Turns the `limit + 1` rows of [`PageRequest::fetch_limit`] into a page.
    ///
    /// The extra row is dropped and only tells whether a next page exists; the
    /// cursor is then the sort key of the last row actually returned, as
    /// `cursor_of` reads it.
    #[must_use]
    pub fn from_rows<C: Cursor>(
        mut rows: Vec<T>,
        request: &PageRequest<C>,
        cursor_of: impl Fn(&T) -> C,
    ) -> Self {
        let limit = usize::from(request.limit());
        if rows.len() <= limit {
            return Self::last(rows);
        }
        rows.truncate(limit);
        let next_cursor = rows.last().map(|row| cursor_of(row).render());
        Self {
            items: rows,
            next_cursor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(limit: Option<&str>, cursor: Option<&str>) -> PageQuery {
        PageQuery {
            limit: limit.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
        }
    }

    #[test]
    fn an_empty_query_uses_the_default_page_size_and_no_cursor() {
        let request = query(None, None)
            .into_request::<Uuid>()
            .expect("must validate");
        assert_eq!(request.limit(), DEFAULT_LIMIT);
        assert_eq!(request.cursor(), None);
    }

    #[test]
    fn the_page_size_is_capped() {
        let error = query(Some("201"), None)
            .into_request::<Uuid>()
            .expect_err("must fail");
        let problem = error.to_problem_details();
        assert_eq!(problem.status, 400);
        assert_eq!(
            problem.errors,
            vec![FieldError::new(
                "limit",
                "must be a whole number between 1 and 200",
            )]
        );

        let accepted = query(Some("200"), None)
            .into_request::<Uuid>()
            .expect("200 is legal");
        assert_eq!(accepted.limit(), MAX_LIMIT);
    }

    #[test]
    fn a_page_size_of_zero_is_rejected() {
        // Zero would answer an empty page forever, which reads as "collection
        // exhausted" and silently truncates a client's iteration.
        let error = query(Some("0"), None)
            .into_request::<Uuid>()
            .expect_err("must fail");
        assert_eq!(error.to_problem_details().errors[0].field, "limit");
    }

    #[test]
    fn an_unparsable_cursor_is_reported_as_a_field_error() {
        let error = query(None, Some("page-2"))
            .into_request::<Uuid>()
            .expect_err("must fail");
        let problem = error.to_problem_details();
        assert_eq!(problem.status, 400);
        assert_eq!(problem.errors[0].field, "cursor");
    }

    #[test]
    fn both_members_are_reported_in_one_answer() {
        // Reporting one member at a time forces a client through as many
        // round trips as it made mistakes.
        let error = query(Some("nope"), Some("nope"))
            .into_request::<Uuid>()
            .expect_err("must fail");
        let fields: Vec<String> = error
            .to_problem_details()
            .errors
            .into_iter()
            .map(|field_error| field_error.field)
            .collect();
        assert_eq!(fields, vec!["limit".to_owned(), "cursor".to_owned()]);
    }

    #[test]
    fn a_cursor_is_carried_through_unchanged() {
        let id = crate::id::new_id();
        let request = query(Some("10"), Some(&id.to_string()))
            .into_request::<Uuid>()
            .expect("must validate");
        assert_eq!(
            request,
            PageRequest::new(10, Some(id)).expect("10 is legal")
        );
    }

    #[test]
    fn a_request_holds_a_page_size_within_the_bounds_whoever_builds_it() {
        // A handler building its own request, rather than reading one from a
        // query string, is held to the same bounds: zero rows would answer an
        // empty page with no cursor — "collection exhausted" while rows remain.
        assert_eq!(PageRequest::<Uuid>::new(0, None), None);
        assert_eq!(PageRequest::<Uuid>::new(MAX_LIMIT + 1, None), None);
        assert_eq!(
            PageRequest::<Uuid>::new(1, None).map(|request| request.limit()),
            Some(1)
        );
        assert_eq!(
            PageRequest::<Uuid>::new(MAX_LIMIT, None).map(|request| request.limit()),
            Some(MAX_LIMIT)
        );
    }

    #[test]
    fn a_cursor_of_identifiers_reads_back_what_it_renders() {
        let id = crate::id::new_id();
        assert_eq!(<Uuid as Cursor>::parse(&id.render()), Some(id));
        assert_eq!(<Uuid as Cursor>::parse("page-2"), None);
    }

    #[test]
    fn one_row_more_than_the_page_is_fetched() {
        let request = PageRequest::<Uuid>::new(50, None).expect("a legal size");
        assert_eq!(request.fetch_limit(), 51);
    }

    #[test]
    fn a_short_read_ends_the_collection() {
        let request = PageRequest::<Uuid>::new(3, None).expect("a legal size");
        let rows: Vec<Uuid> = (0..3).map(|_| crate::id::new_id()).collect();
        let page = Page::from_rows(rows.clone(), &request, |id| *id);
        assert_eq!(page.items, rows);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn the_extra_row_is_dropped_and_becomes_the_next_cursor() {
        let request = PageRequest::<Uuid>::new(3, None).expect("a legal size");
        let rows: Vec<Uuid> = (0..4).map(|_| crate::id::new_id()).collect();
        let page = Page::from_rows(rows.clone(), &request, |id| *id);

        assert_eq!(page.items, rows[..3]);
        assert_eq!(
            page.next_cursor,
            Some(rows[2].to_string()),
            "the cursor must be the last item served, not the one held back"
        );
    }

    #[test]
    fn a_page_serialises_under_the_agreed_member_names() {
        let page = Page::last(vec![1_u8, 2]);
        assert_eq!(
            serde_json::to_string(&page).expect("must serialise"),
            r#"{"items":[1,2],"next_cursor":null}"#
        );
    }
}
