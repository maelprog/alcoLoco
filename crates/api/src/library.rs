//! `/library/items` — the base library of SPEC.md §5.4, searched by name.
//!
//! The library ships with the application and is read-only in V1: its rows are
//! written by a migration (issue #10), and this module only reads them. Writing
//! to it is #32's.
//!
//! **What the search matches.** The text typed is looked for *anywhere* in the
//! name, not only at its start: « orange » must find « Jus d'orange », and
//! « blonde » « Bière blonde ». Both sides go through
//! `public.library_item_search_key`, the key of #10 that ignores case and
//! accents, so « BIERE » finds « Bière blonde » and « cachaca » « Cachaça ».
//! The containment test is `strpos`, not `LIKE`: a `%` or a `_` typed by the
//! user is a character to look for, never a wildcard, and needs no escaping.
//!
//! That choice gives up the index #10 built for the prefix form
//! (`key LIKE typed || '%'`): a containment search reads the whole table. The
//! table is the read-only library shipped with the application — a few dozen
//! rows — so the scan is the cheaper plan anyway, and no trigram index (a new
//! extension) is added for it.
//!
//! **What an empty search returns.** No `q`, or a blank one, filters nothing:
//! the first page of the whole library — or of one category — in the order the
//! migration lists it, which is the order of the identifiers. That is the
//! default list the issue asks for, and the cursor pagination of
//! [`crate::pagination`] walks it like any other collection.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::{ApiError, FieldError};
use crate::extract::Query;
use crate::pagination::{Page, PageQuery, PageRequest};
use crate::{AppState, ProblemDetails};

/// Where the collection is served.
pub const COLLECTION_PATH: &str = "/library/items";

/// Category of a library item (SPEC.md §5.4).
///
/// The two variants are the two labels of the `library_item_category` enum of
/// the database; [`Category::as_sql`] is the single place that spelling is
/// written, and a test ties it to the migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// An alcoholic drink, with its usual degree.
    Alcohol,
    /// A soft drink, at 0 %.
    Soft,
}

impl Category {
    /// Every variant, in the order the database enum declares them.
    pub const ALL: [Self; 2] = [Self::Alcohol, Self::Soft];

    /// Label of this variant in the PostgreSQL `library_item_category` enum,
    /// which is also its spelling in JSON and in `?category=`.
    #[must_use]
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Alcohol => "alcohol",
            Self::Soft => "soft",
        }
    }

    /// Reads back a label produced by [`Category::as_sql`].
    #[must_use]
    pub fn from_sql(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_sql() == label)
    }
}

/// One entry of the base library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LibraryItem {
    /// Identifier of the item.
    pub id: Uuid,
    /// Name of the item, as shipped.
    pub name: String,
    /// Usual degree, in percent of alcohol by volume. A default: the drink
    /// component copies it and the user corrects it for the bottle in hand.
    pub default_abv_percent: f64,
    /// Alcohol or soft drink.
    pub category: Category,
}

/// The `?q=&category=` pair, exactly as it arrives.
///
/// Read as text and validated by [`SearchQuery::into_search`], for the reason
/// [`PageQuery`] is: an unknown category is reported as a field error inside a
/// problem document, together with whatever else is wrong with the request.
#[derive(Debug, Default, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Text looked for in the names, ignoring case and accents. Blank or
    /// absent: every item.
    pub q: Option<String>,
    /// `alcohol` or `soft`. Absent: both.
    pub category: Option<String>,
}

/// A validated search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// Trimmed text to look for, `None` when there is nothing to look for.
    pub text: Option<String>,
    /// Category to keep, `None` for both.
    pub category: Option<Category>,
}

impl SearchQuery {
    /// Validates the pair, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns the field errors of each member that could not be used.
    pub fn into_search(self) -> Result<Search, Vec<FieldError>> {
        let mut errors = Vec::new();

        let text = match blank_to_none(self.q) {
            Some(text) if text.contains('\0') => {
                errors.push(FieldError::new("q", "must not contain a NUL character"));
                None
            }
            text => text,
        };

        let category = match blank_to_none(self.category) {
            None => None,
            Some(label) => {
                let category = Category::from_sql(&label);
                if category.is_none() {
                    let legal: Vec<&str> =
                        Category::ALL.into_iter().map(Category::as_sql).collect();
                    errors.push(FieldError::new(
                        "category",
                        format!("must be one of: {}", legal.join(", ")),
                    ));
                }
                category
            }
        };

        if errors.is_empty() {
            Ok(Search { text, category })
        } else {
            Err(errors)
        }
    }
}

/// The trimmed value, or `None` when it is absent or blank.
fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Lists the library items matching a search, one page at a time.
#[utoipa::path(
    get,
    path = COLLECTION_PATH,
    tag = "library",
    params(SearchQuery, PageQuery),
    responses(
        (status = 200, description = "One page of matching items, in library order", body = Page<LibraryItem>),
        (status = 400, description = "Unusable `q`, `category`, `limit` or `cursor`", body = ProblemDetails),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Query(search): Query<SearchQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<LibraryItem>>, ApiError> {
    let (search, page) = validate(search, page)?;
    Ok(Json(store_list(&state.pool, &search, page).await?))
}

/// Validates the search and the page together, so that one answer lists every
/// member a client got wrong.
fn validate(search: SearchQuery, page: PageQuery) -> Result<(Search, PageRequest), ApiError> {
    match (search.into_search(), page.into_parts()) {
        (Ok(search), Ok(page)) => Ok((search, page)),
        (search, page) => {
            let mut errors = search.err().unwrap_or_default();
            errors.extend(page.err().unwrap_or_default());
            Err(ApiError::validation(errors))
        }
    }
}

/// One page of the library items matching `$1` and `$2`, resumed after `$3`.
///
/// Every parameter may be null, and is then no filter: `$1` is the text typed,
/// `$2` the label of a category, `$3` the cursor. They are cast because a bound
/// `None` arrives as an untyped null. Relations, functions and types are schema
/// qualified, as in the profile store. Ordering by `id` is the order the
/// migration lists the library in (its identifiers are UUID v7, numbered in
/// that order).
const SELECT_PAGE: &str = "
    SELECT i.id,
           i.name,
           i.default_abv_percent,
           i.category::pg_catalog.text AS category
    FROM public.library_item AS i
    WHERE ($1::pg_catalog.text IS NULL
           OR pg_catalog.strpos(public.library_item_search_key(i.name),
                                public.library_item_search_key($1::pg_catalog.text)) > 0)
      AND ($2::pg_catalog.text IS NULL OR i.category::pg_catalog.text = $2::pg_catalog.text)
      AND ($3::pg_catalog.uuid IS NULL OR i.id > $3::pg_catalog.uuid)
    ORDER BY i.id
    LIMIT $4
";

/// Runs the search.
async fn store_list(
    pool: &PgPool,
    search: &Search,
    request: PageRequest,
) -> Result<Page<LibraryItem>, ApiError> {
    let rows = sqlx::query(SELECT_PAGE)
        .bind(search.text.as_deref())
        .bind(search.category.map(Category::as_sql))
        .bind(request.cursor().copied())
        .bind(request.fetch_limit())
        .fetch_all(pool)
        .await?;

    let items = rows
        .iter()
        .map(item_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Page::from_rows(items, &request, |item| item.id))
}

/// Reads one row of [`SELECT_PAGE`].
fn item_from_row(row: &PgRow) -> Result<LibraryItem, ApiError> {
    let label: String = row.try_get("category")?;
    let category = Category::from_sql(&label)
        .ok_or_else(|| ApiError::internal(format!("unknown library item category `{label}`")))?;
    Ok(LibraryItem {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        default_abv_percent: row.try_get("default_abv_percent")?,
        category,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(q: Option<&str>, category: Option<&str>) -> SearchQuery {
        SearchQuery {
            q: q.map(str::to_owned),
            category: category.map(str::to_owned),
        }
    }

    fn fields(errors: &[FieldError]) -> Vec<&str> {
        errors.iter().map(|error| error.field.as_str()).collect()
    }

    #[test]
    fn an_absent_or_blank_search_filters_nothing() {
        for q in [None, Some(""), Some("   "), Some("\t\n")] {
            assert_eq!(
                query(q, None).into_search().expect("must validate"),
                Search {
                    text: None,
                    category: None
                },
                "q = {q:?}"
            );
        }
    }

    #[test]
    fn the_text_is_trimmed_and_otherwise_kept_as_typed() {
        // Case and accents are the database's to fold, through the same key
        // the names go through; folding them here as well would be a second
        // definition of the key, free to drift from the first.
        let search = query(Some("  Bière Blonde "), None)
            .into_search()
            .expect("must validate");
        assert_eq!(search.text.as_deref(), Some("Bière Blonde"));
    }

    #[test]
    fn wildcard_characters_are_kept_as_typed() {
        let search = query(Some("50%_"), None)
            .into_search()
            .expect("must validate");
        assert_eq!(search.text.as_deref(), Some("50%_"));
    }

    #[test]
    fn a_nul_character_is_a_field_error_and_not_a_server_failure() {
        // PostgreSQL refuses U+0000 in a text value; let through, it would come
        // back as a 500 for what is the client's mistake.
        let errors = query(Some("vod\0ka"), None)
            .into_search()
            .expect_err("must fail");
        assert_eq!(fields(&errors), vec!["q"]);
    }

    #[test]
    fn each_category_is_read_under_its_sql_label() {
        for category in Category::ALL {
            let search = query(None, Some(category.as_sql()))
                .into_search()
                .expect("must validate");
            assert_eq!(search.category, Some(category));
        }
    }

    #[test]
    fn a_blank_category_is_no_filter() {
        let search = query(None, Some(" ")).into_search().expect("must validate");
        assert_eq!(search.category, None);
    }

    #[test]
    fn an_unknown_category_is_a_field_error_naming_the_legal_ones() {
        for label in ["beer", "Alcohol", "SOFT", "alcool"] {
            let errors = query(None, Some(label))
                .into_search()
                .expect_err("must fail");
            assert_eq!(
                errors,
                vec![FieldError::new("category", "must be one of: alcohol, soft")],
                "category = {label:?}"
            );
        }
    }

    #[test]
    fn both_members_are_reported_in_one_answer() {
        let errors = query(Some("\0"), Some("beer"))
            .into_search()
            .expect_err("must fail");
        assert_eq!(fields(&errors), vec!["q", "category"]);
    }

    #[test]
    fn the_search_and_the_page_are_reported_in_one_answer() {
        let page = PageQuery {
            limit: Some("0".to_owned()),
            cursor: Some("nope".to_owned()),
        };
        let error = validate(query(Some("\0"), Some("beer")), page).expect_err("must fail");
        let problem = error.to_problem_details();
        assert_eq!(problem.status, 400);
        let fields: Vec<String> = problem.errors.into_iter().map(|e| e.field).collect();
        assert_eq!(fields, vec!["q", "category", "limit", "cursor"]);
    }

    #[test]
    fn a_page_error_alone_is_reported_as_it_was() {
        let page = PageQuery {
            limit: Some("201".to_owned()),
            cursor: None,
        };
        let error = validate(query(Some("gin"), None), page).expect_err("must fail");
        let fields: Vec<String> = error
            .to_problem_details()
            .errors
            .into_iter()
            .map(|e| e.field)
            .collect();
        assert_eq!(fields, vec!["limit"]);
    }

    #[test]
    fn a_category_travels_on_the_wire_under_its_sql_label() {
        for category in Category::ALL {
            let rendered = serde_json::to_value(category).expect("must serialise");
            assert_eq!(rendered, category.as_sql());
            assert_eq!(Category::from_sql(category.as_sql()), Some(category));
        }
    }

    #[test]
    fn an_item_serialises_under_the_column_names() {
        let item = LibraryItem {
            id: Uuid::nil(),
            name: "Vodka".to_owned(),
            default_abv_percent: 40.0,
            category: Category::Alcohol,
        };
        assert_eq!(
            serde_json::to_value(&item).expect("must serialise"),
            serde_json::json!({
                "id": "00000000-0000-0000-0000-000000000000",
                "name": "Vodka",
                "default_abv_percent": 40.0,
                "category": "alcohol",
            })
        );
    }
}
