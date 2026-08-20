//! Cursor pagination, shared by every collection the API will expose.
//!
//! The contract is fixed here once: a request carries `?limit=&cursor=`, an
//! answer carries `{ "items": [...], "next_cursor": <string|null> }`. `limit`
//! defaults to [`DEFAULT_LIMIT`] and is capped at [`MAX_LIMIT`].
//!
//! The cursor is the identifier of the last row of the page. That works because
//! identifiers are UUID v7 (see [`crate::id`]): they sort in creation order, so
//! `WHERE id > $cursor ORDER BY id` resumes exactly where the previous page
//! stopped. An `OFFSET` would not: a row inserted meanwhile shifts every
//! subsequent offset and makes a row appear twice or not at all.

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
/// deserialiser before the handler is ever reached.
#[derive(Debug, Default, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// Number of items asked for, at most [`MAX_LIMIT`].
    pub limit: Option<String>,
    /// Identifier of the last item of the previous page.
    pub cursor: Option<String>,
}

/// A validated page request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    /// Number of items to return, within `1..=MAX_LIMIT`.
    pub limit: u16,
    /// Exclusive lower bound on the identifiers to return.
    pub cursor: Option<Uuid>,
}

impl PageQuery {
    /// Validates the pair, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that could not be used.
    pub fn into_request(self) -> Result<PageRequest, ApiError> {
        let mut errors = Vec::new();

        let limit = match self
            .limit
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => DEFAULT_LIMIT,
            Some(text) => match text.parse::<u16>() {
                Ok(value) if (1..=MAX_LIMIT).contains(&value) => value,
                _ => {
                    errors.push(FieldError::new(
                        "limit",
                        format!("must be a whole number between 1 and {MAX_LIMIT}"),
                    ));
                    DEFAULT_LIMIT
                }
            },
        };

        let cursor = match self
            .cursor
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(text) => match text.parse::<Uuid>() {
                Ok(id) => Some(id),
                Err(_) => {
                    errors.push(FieldError::new(
                        "cursor",
                        "must be the `next_cursor` of a previous page",
                    ));
                    None
                }
            },
        };

        if errors.is_empty() {
            Ok(PageRequest { limit, cursor })
        } else {
            Err(ApiError::validation(errors))
        }
    }
}

impl PageRequest {
    /// How many rows to ask the database for: one more than the page size, so
    /// that the presence of a next page is known without a second query.
    #[must_use]
    pub const fn fetch_limit(self) -> i64 {
        self.limit as i64 + 1
    }
}

/// One page of a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    /// The items of this page, in identifier order.
    pub items: Vec<T>,
    /// Cursor to pass back to obtain the next page, `null` on the last one.
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
    /// cursor is then the identifier of the last row actually returned.
    #[must_use]
    pub fn from_rows(mut rows: Vec<T>, request: PageRequest, id_of: impl Fn(&T) -> Uuid) -> Self {
        if rows.len() as i64 <= i64::from(request.limit) {
            return Self::last(rows);
        }
        rows.truncate(request.limit as usize);
        let next_cursor = rows.last().map(|row| id_of(row).to_string());
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
        let request = query(None, None).into_request().expect("must validate");
        assert_eq!(request.limit, DEFAULT_LIMIT);
        assert_eq!(request.cursor, None);
    }

    #[test]
    fn the_page_size_is_capped() {
        let error = query(Some("201"), None)
            .into_request()
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
            .into_request()
            .expect("200 is legal");
        assert_eq!(accepted.limit, MAX_LIMIT);
    }

    #[test]
    fn a_page_size_of_zero_is_rejected() {
        // Zero would answer an empty page forever, which reads as "collection
        // exhausted" and silently truncates a client's iteration.
        let error = query(Some("0"), None)
            .into_request()
            .expect_err("must fail");
        assert_eq!(error.to_problem_details().errors[0].field, "limit");
    }

    #[test]
    fn an_unparsable_cursor_is_reported_as_a_field_error() {
        let error = query(None, Some("page-2"))
            .into_request()
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
            .into_request()
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
            .into_request()
            .expect("must validate");
        assert_eq!(
            request,
            PageRequest {
                limit: 10,
                cursor: Some(id)
            }
        );
    }

    #[test]
    fn one_row_more_than_the_page_is_fetched() {
        let request = PageRequest {
            limit: 50,
            cursor: None,
        };
        assert_eq!(request.fetch_limit(), 51);
    }

    #[test]
    fn a_short_read_ends_the_collection() {
        let request = PageRequest {
            limit: 3,
            cursor: None,
        };
        let rows: Vec<Uuid> = (0..3).map(|_| crate::id::new_id()).collect();
        let page = Page::from_rows(rows.clone(), request, |id| *id);
        assert_eq!(page.items, rows);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn the_extra_row_is_dropped_and_becomes_the_next_cursor() {
        let request = PageRequest {
            limit: 3,
            cursor: None,
        };
        let rows: Vec<Uuid> = (0..4).map(|_| crate::id::new_id()).collect();
        let page = Page::from_rows(rows.clone(), request, |id| *id);

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
