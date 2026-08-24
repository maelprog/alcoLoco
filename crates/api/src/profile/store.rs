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
//! That rule is held by this file rather than by a list someone remembers to
//! keep up to date: the tests read the source at compile time, recover every
//! string constant it declares, and require every call written `sqlx::query(…)`
//! or `sqlx::raw_sql(…)` to be handed the *name* of one. What that buys is
//! **visibility, not impossibility**. A statement assembled at run time is
//! caught in the shapes the guards recognise, and the shapes they do not
//! recognise are written down one by one at the top of the test module: reading
//! Rust exactly would take a Rust lexer, and reading SQL exactly an SQL parser,
//! neither of which belongs in a test module. A guard that names its blind
//! spots is worth more than one that promises they do not exist. The two
//! sibling files of the module declare no statement at all, and a test holds
//! them to it — which is what makes a file-scoped guard cover the feature.
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
    //! What these guards are worth, and where they stop.
    //!
    //! They read this file as text and decide two things about it: that every
    //! statement is a named constant, and that every name a statement puts in
    //! relation or type position is schema qualified. Deciding either exactly
    //! would take a Rust lexer and an SQL parser. What is here instead is a
    //! reader held to the shapes it claims, by tests of its own — so these
    //! guards make a mistake **visible**, they do not make it **impossible**,
    //! and the difference is this list.
    //!
    //! Four ways to reach sqlx without the reader seeing a call at all. Each
    //! was written into this file, compiled and run on #6; each left all guards
    //! green, and none of them is refused:
    //!
    //! - `use sqlx::query;` and then `query(&format!("…"))` — the reader looks
    //!   for the path, so an import that drops it hides the call;
    //! - `use sqlx::query as run;` and then `run(&statement)` — the same, under
    //!   a name the reader has never heard of;
    //! - `sqlx::QueryBuilder::<Postgres>::new(format!("…")).build()` — a third
    //!   entry point, and not one of the two the reader knows;
    //! - `use sqlx::Executor;` and then `pool.execute(statement)` — a method on
    //!   the pool, with no sqlx path at the call site whatsoever.
    //!
    //! Two more get past with the call in plain sight:
    //!
    //! - `concat!("… FROM ", "public.profile")` reaches the reader as its
    //!   pieces, so a relation name split across two of them follows no keyword
    //!   in either;
    //! - a name written as a quoted identifier — `FROM "profile"` — is not read
    //!   as a name, so nothing checks that it is qualified.
    //!
    //! Closing those was weighed and declined on #6. Each is one more spelling
    //! to recognise, and this module has already spent four turns moving the
    //! same hole one notch at a time: a hand-copied list, then the spelling of
    //! a declaration, then the spelling of a call and the width of a space.
    //! What the guards do buy is worth stating as plainly: every statement this
    //! file declares is read in every shape a declaration can be written in,
    //! every keyword is matched whatever whitespace follows it, and the shape a
    //! hurried change actually takes — `sqlx::query(&format!(…))` — turns them
    //! red.
    //!
    //! Two refusals below are deliberate rather than exact: they turn down
    //! legitimate SQL and legitimate Rust that this module does not use, on the
    //! grounds that admitting them would take a parser. Both say so where they
    //! are made — see [`no_statement_of_this_module_carries_an_interpolation_marker`]
    //! on braces, and [`every_statement_of_this_module_is_a_named_constant`] on
    //! qualified paths.

    use super::*;

    /// This file, read at compile time.
    ///
    /// Every guard below works from it rather than from a list of statements
    /// transcribed by hand. A transcribed list only ever guards what someone
    /// remembered to add to it: the first review of this module shipped one, and
    /// a function appended below it — building its SQL by interpolation and
    /// naming an unqualified relation — passed all three guards without a word.
    /// The same reasoning already applies to `tests/front_contract.rs`, which
    /// reads the `slug` match arms instead of listing them.
    const SOURCE: &str = include_str!("store.rs");

    /// The value of every string constant this file declares.
    ///
    /// The statements are exactly those constants, and
    /// [`every_statement_of_this_module_is_a_named_constant`] is what keeps that
    /// true. Non-SQL constants are swept in as well; they satisfy the guards
    /// vacuously, and an allowlist would be one more thing to forget.
    fn declared_string_constants() -> Vec<String> {
        string_literals_of_declarations(SOURCE)
    }

    /// Every string literal a `const` or a `static` of `source` introduces,
    /// whatever its declaration is spelled like.
    ///
    /// The reader this replaced looked for one spelling of a declaration — the
    /// bytes `: &str = "` — and so guarded the shapes someone had thought of
    /// rather than the ones the file holds: a statement written `r#"…"#`, the
    /// natural shape for SQL of several lines, entered no list at all, and the
    /// guards below iterated straight past it. So did `&'static str`, a value
    /// rustfmt wrapped to the next line, and punctuation written without spaces
    /// around it. Adding those spellings to a list of spellings would move the
    /// hole one notch up, so there is no list here: the source is walked token
    /// by token — comments, character literals and lifetimes told apart from the
    /// strings they resemble — and every literal between a `const` or `static`
    /// keyword and the `;` that closes its item is taken, whatever the declared
    /// type says. [`the_reader_recognises_every_form_a_declaration_can_take`]
    /// holds it to that.
    ///
    /// A `const fn` sweeps the literals of its first statement in as well. That
    /// errs towards guarding too much, which is the harmless direction, and
    /// this module declares none.
    ///
    /// What no declaration form reaches — `concat!` among it — is listed with
    /// the other blind spots at the top of this module, where a reader looking
    /// for what these guards are worth will find all of them together.
    fn string_literals_of_declarations(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        // The bracket depth the open declaration started at, if one is open.
        // Depth is what tells the `;` that ends an item from the one inside
        // `[&str; 3]`, which used to end it three statements too early: the
        // first shape this reader was written in read a list of statements as
        // no statement at all.
        let mut declared_at_depth: Option<u32> = None;
        let mut depth = 0_u32;
        let mut at = 0;
        while at < source.len() {
            let rest = &source[at..];
            if let Some(length) = comment_length(rest) {
                at += length;
            } else if let Some(length) = character_or_lifetime_length(rest) {
                // Both open with a quote and neither opens a string: `'"'` is a
                // quote that starts nothing, and the `'static` of
                // `&'static str` is not the keyword `static`.
                at += length;
            } else if let Some((literal, length)) = string_literal(rest) {
                if declared_at_depth.is_some() {
                    literals.push(literal);
                }
                at += length;
            } else if let Some(length) = word_length(rest) {
                if matches!(&rest[..length], "const" | "static") {
                    declared_at_depth = Some(depth);
                }
                at += length;
            } else {
                match rest.as_bytes()[0] {
                    b'(' | b'[' => depth += 1,
                    b')' | b']' => depth = depth.saturating_sub(1),
                    // Braces are left out of the count on purpose: a `const fn`
                    // body ends no item with a `;`, so counting them would hold
                    // the declaration open over the rest of the file.
                    b';' if declared_at_depth.is_some_and(|opened| depth <= opened) => {
                        declared_at_depth = None;
                    }
                    _ => {}
                }
                at += 1;
            }
        }
        literals
    }

    /// The length of the comment `rest` opens, if it opens one.
    ///
    /// Taken whole: a comment may hold anything, an unbalanced quote included.
    fn comment_length(rest: &str) -> Option<usize> {
        if rest.starts_with("//") {
            return Some(rest.find('\n').map_or(rest.len(), |end| end + 1));
        }
        if !rest.starts_with("/*") {
            return None;
        }
        let bytes = rest.as_bytes();
        let mut depth = 0_u32;
        let mut at = 0;
        while at + 1 < bytes.len() {
            match &bytes[at..at + 2] {
                b"/*" => {
                    depth += 1;
                    at += 2;
                }
                b"*/" => {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        return Some(at);
                    }
                }
                _ => at += 1,
            }
        }
        Some(rest.len())
    }

    /// The length of the character literal or lifetime `rest` opens.
    ///
    /// A lifetime is a quote and a name with no closing quote after it, which is
    /// what tells `'static` from `'s'`.
    fn character_or_lifetime_length(rest: &str) -> Option<usize> {
        let after_quote = rest.strip_prefix('\'')?;
        let name = after_quote.len()
            - after_quote
                .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
                .len();
        if name > 0
            && !after_quote.starts_with(|c: char| c.is_ascii_digit())
            && !after_quote[name..].starts_with('\'')
        {
            return Some(1 + name);
        }
        let bytes = rest.as_bytes();
        let mut at = 1;
        while at < bytes.len() {
            match bytes[at] {
                // Escape sequences are ASCII, so this lands on a boundary.
                b'\\' => at += 2,
                b'\'' => return Some(at + 1),
                _ => at += 1,
            }
        }
        Some(rest.len())
    }

    /// The content of the string literal `rest` opens, and its length.
    ///
    /// Ordinary, byte and raw spellings alike. The escape sequences of an
    /// ordinary literal are decoded, because a statement may be written
    /// `"SELECT 1 FROM\n    public.profile"` as readily as across three lines
    /// and the guards have to read the same whitespace in both: left as
    /// written, that `\n` is a backslash and a letter, and the token after
    /// `FROM` is neither of them a name. A raw string carries no escapes, so
    /// its bytes are its content.
    fn string_literal(rest: &str) -> Option<(String, usize)> {
        let body = rest.strip_prefix('b').unwrap_or(rest);
        let marker = rest.len() - body.len();
        let Some(raw) = body.strip_prefix('r') else {
            let inside = body.strip_prefix('"')?;
            let bytes = inside.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                match bytes[at] {
                    b'\\' => at += 2,
                    b'"' => {
                        return Some((unescaped(&inside[..at]), marker + 1 + at + 1));
                    }
                    _ => at += 1,
                }
            }
            panic!("unterminated string literal in {}", file!())
        };
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let inside = raw[hashes..].strip_prefix('"')?;
        let closing = format!("{}{}", '"', "#".repeat(hashes));
        let end = inside
            .find(closing.as_str())
            .unwrap_or_else(|| panic!("unterminated raw string literal in {}", file!()));
        Some((
            inside[..end].to_owned(),
            marker + 1 + hashes + 1 + end + closing.len(),
        ))
    }

    /// What `escaped` stands for once the escape sequences Rust writes are
    /// decoded.
    ///
    /// The set of them is finite and written down in the reference, which is
    /// what makes this exact rather than one more guess at a spelling.
    fn unescaped(escaped: &str) -> String {
        let mut text = String::with_capacity(escaped.len());
        let mut characters = escaped.chars().peekable();
        while let Some(character) = characters.next() {
            if character != '\\' {
                text.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => text.push('\n'),
                Some('r') => text.push('\r'),
                Some('t') => text.push('\t'),
                Some('0') => text.push('\0'),
                Some('x') => {
                    let digits: String = (0..2).filter_map(|_| characters.next()).collect();
                    push_code_point(&mut text, &digits);
                }
                Some('u') => {
                    let mut digits = String::new();
                    for digit in characters.by_ref() {
                        if digit == '}' {
                            break;
                        }
                        if digit != '{' {
                            digits.push(digit);
                        }
                    }
                    push_code_point(&mut text, &digits);
                }
                // A backslash ending a line eats the break and the indentation
                // that follows it, so both spellings of a wrapped statement
                // read alike.
                Some('\n') => while characters.next_if(|c| c.is_whitespace()).is_some() {},
                // `\\`, `\'` and `\"` stand for themselves.
                Some(other) => text.push(other),
                None => text.push('\\'),
            }
        }
        text
    }

    /// Appends the character `digits` names in hexadecimal, or the digits
    /// themselves when they name none.
    fn push_code_point(text: &mut String, digits: &str) {
        match u32::from_str_radix(digits, 16)
            .ok()
            .and_then(char::from_u32)
        {
            Some(character) => text.push(character),
            None => text.push_str(digits),
        }
    }

    /// The length of the word `rest` opens, if it opens one.
    fn word_length(rest: &str) -> Option<usize> {
        let length = rest.len()
            - rest
                .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
                .len();
        (length > 0).then_some(length)
    }

    /// This file with its comments and its string literals blanked out.
    ///
    /// [`first_arguments_of`] searches text, and text includes what is written
    /// *about* the code. Before this, a comment saying what never to write —
    /// `sqlx::query(&format!(…))`, spelled out as the counter-example it is —
    /// turned [`every_statement_of_this_module_is_a_named_constant`] red, and
    /// so would a test fixture quoting the same shape. A guard that punishes
    /// the documentation of its own rule gets the documentation deleted.
    ///
    /// Blanking rather than removing keeps the code at its own offsets, and
    /// blanking string literals is the second half of the rule the guard
    /// states: SQL handed to a constructor as a literal is not the name of a
    /// constant, and it now reads as the empty argument it is.
    fn code_of(source: &str) -> String {
        let mut code = String::with_capacity(source.len());
        let mut at = 0;
        while at < source.len() {
            let rest = &source[at..];
            let blanked =
                comment_length(rest).or_else(|| string_literal(rest).map(|(_, length)| length));
            if let Some(length) = blanked {
                code.push_str(&" ".repeat(length));
                at += length;
                continue;
            }
            // A character literal is code, and `'"'` must not be read as the
            // start of a string.
            let length = character_or_lifetime_length(rest)
                .unwrap_or_else(|| rest.chars().next().map_or(1, char::len_utf8));
            code.push_str(&rest[..length]);
            at += length;
        }
        code
    }

    /// The text each call to an sqlx statement constructor passes first.
    ///
    /// `prefix` is the path up to the constructor family, e.g. the sqlx query
    /// builders or the raw-SQL entry point. The search runs on [`code_of`] this
    /// file, so the word occurring in a comment or inside a string is no call;
    /// the guard that reads this insists on finding some all the same.
    fn first_arguments_of(prefix: &str) -> Vec<String> {
        let code = code_of(SOURCE);
        let mut arguments = Vec::new();
        let mut from = 0;
        while let Some(at) = code[from..].find(prefix) {
            let after_prefix = from + at + prefix.len();
            from = after_prefix;
            // The rest of the function name: `_scalar`, `_as`, or nothing.
            let rest = code[after_prefix..]
                .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
            // An optional turbofish, whose own parentheses must not be mistaken
            // for the call's: `query_as::<_, (i64,)>(…)`.
            let rest = match rest.strip_prefix("::<") {
                None => rest,
                Some(generics) => match generics.find(">(") {
                    None => continue,
                    Some(end) => &generics[end + 1..],
                },
            };
            let Some(inside) = rest.strip_prefix('(') else {
                continue;
            };
            let mut depth = 0_i32;
            let mut argument = String::new();
            for character in inside.chars() {
                match character {
                    '(' | '[' => depth += 1,
                    ')' | ']' if depth == 0 => break,
                    ')' | ']' => depth -= 1,
                    ',' if depth == 0 => break,
                    _ => {}
                }
                argument.push(character);
            }
            arguments.push(argument.trim().to_owned());
        }
        arguments
    }

    /// The two sqlx entry points that take SQL, spelled in pieces so that naming
    /// them here does not look like a call to [`first_arguments_of`].
    fn sql_constructor_prefixes() -> [String; 2] {
        let sqlx = "sqlx";
        [format!("{sqlx}::query"), format!("{sqlx}::raw_sql")]
    }

    #[test]
    fn every_statement_of_this_module_is_a_named_constant() {
        // The guard the other two rest on: a call the reader sees is handed
        // the name of a constant and nothing else. `format!("… {id} …")`, a
        // `String` built above the call, a borrowed local, a statement written
        // inline as a literal — none of them is an upper-case identifier, so
        // none of them gets past here. What the reader does not see is the list
        // at the top of this module: this makes the mistake visible, it does
        // not make it impossible.
        //
        // A qualified path such as `Q::SQL` is refused as well, deliberately
        // and not by oversight: the reader reads this file, so a statement
        // named through a path could be declared somewhere no guard of this
        // module would ever inspect it, and the file-scoped guarantee would
        // quietly stop covering the feature. A constant this module runs is
        // declared in this module.
        let mut seen = 0;
        for prefix in sql_constructor_prefixes() {
            for argument in first_arguments_of(&prefix) {
                seen += 1;
                assert!(
                    !argument.is_empty()
                        && argument
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                        && argument.starts_with(|c: char| c.is_ascii_uppercase()),
                    "a statement is built rather than named: `{argument}`"
                );
            }
        }
        assert!(
            seen >= 5,
            "only {seen} statement constructor call(s) found; the reader has lost \
             the shape of this file and is guarding nothing"
        );
    }

    #[test]
    fn no_sql_of_the_profile_module_lives_outside_this_file() {
        // What makes the file-scoped guards above cover the whole feature: the
        // handlers, the payload types and the validation touch no database at
        // all, so every statement a profile write runs is one of the constants
        // read here.
        for (name, source) in [
            ("mod.rs", include_str!("mod.rs")),
            ("model.rs", include_str!("model.rs")),
            ("validation.rs", include_str!("validation.rs")),
        ] {
            let sqlx = "sqlx";
            assert!(
                !source.contains(&format!("{sqlx}::")),
                "`{name}` reaches for the database; the guards of store.rs do not read it"
            );
        }
    }

    /// The tokens of an SQL statement: its names, its punctuation, and each of
    /// its string literals taken whole.
    ///
    /// Whitespace separates and nothing more, which is the whole point. The
    /// reader this replaced looked for a keyword written with exactly one space
    /// after it, so `FROM` at the end of a line — the shape every statement of
    /// this file is written in — introduced no name at all as far as the guards
    /// could see. Whitespace is a closed set, so tokenising is exact here in a
    /// way that guessing at Rust spellings never was.
    fn sql_tokens(statement: &str) -> Vec<&str> {
        let bytes = statement.as_bytes();
        let mut tokens = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let start = at;
            if bytes[at].is_ascii_whitespace() {
                at += 1;
                continue;
            } else if bytes[at] == b'\'' {
                // One token: the words inside a literal are text, not names.
                at += 1;
                while at < bytes.len() {
                    let quote = bytes[at] == b'\'';
                    at += 1;
                    // `''` is an escaped quote and not the end of the literal.
                    if quote && bytes.get(at) != Some(&b'\'') {
                        break;
                    }
                }
            } else if bytes[at] == b':' && bytes.get(at + 1) == Some(&b':') {
                at += 2;
            } else if is_name_byte(bytes[at]) {
                while at < bytes.len() && is_name_byte(bytes[at]) {
                    at += 1;
                }
            } else {
                // A whole character, so that the slicing below stays on a
                // boundary whatever a statement is written in.
                at += statement[at..].chars().next().map_or(1, char::len_utf8);
            }
            tokens.push(&statement[start..at]);
        }
        tokens
    }

    /// Whether `byte` can appear inside a name: an identifier, a qualified one,
    /// or a placeholder such as `$1`.
    fn is_name_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$')
    }

    /// Whether `token` reads as a name rather than as punctuation or a literal.
    fn is_a_name(token: &str) -> bool {
        token.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
    }

    /// The name each occurrence of `keyword` introduces in `statement`.
    ///
    /// `keyword` is matched as a whole token and against a whole token, so what
    /// separates it from the name it introduces — one space, several, a
    /// tabulation, the line break this file wraps its statements with — makes
    /// no difference, and neither does its case.
    ///
    /// `LATERAL` is skipped rather than returned: `JOIN LATERAL public.f(…)`
    /// names `public.f`, and the `LATERAL` keyword picks it up on its own pass.
    /// So is anything that is not a name at all: `FROM (SELECT …)` opens a
    /// subquery, whose own `FROM` is read on its own.
    fn names_after(statement: &str, keyword: &str) -> Vec<String> {
        let tokens = sql_tokens(statement);
        let mut names = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if !token.eq_ignore_ascii_case(keyword) {
                continue;
            }
            let Some(name) = tokens.get(index + 1) else {
                continue;
            };
            if !name.eq_ignore_ascii_case("LATERAL") && is_a_name(name) {
                names.push((*name).to_owned());
            }
        }
        names
    }

    /// The type each cast of `statement` names, in both spellings PostgreSQL
    /// accepts: `value::type`, whatever whitespace surrounds the colons, and
    /// the standard `CAST(value AS type)`, which carries no colons at all.
    ///
    /// Only the `AS` of a `CAST` is read; the one that names a column or a
    /// table alias introduces no type.
    fn cast_target_names(statement: &str) -> Vec<String> {
        let tokens = sql_tokens(statement);
        let mut names = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let target = if *token == "::" {
                tokens.get(index + 1).copied()
            } else if token.eq_ignore_ascii_case("CAST") {
                type_named_by_cast(&tokens[index + 1..])
            } else {
                None
            };
            if let Some(name) = target.filter(|name| is_a_name(name)) {
                names.push(name.to_owned());
            }
        }
        names
    }

    /// The type named by the `CAST` whose arguments `tokens` open.
    fn type_named_by_cast<'a>(tokens: &[&'a str]) -> Option<&'a str> {
        if tokens.first() != Some(&"(") {
            return None;
        }
        let mut depth = 0_u32;
        for (index, token) in tokens.iter().enumerate() {
            match *token {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        return None;
                    }
                }
                _ if depth == 1 && token.eq_ignore_ascii_case("AS") => {
                    return tokens.get(index + 1).copied();
                }
                _ => {}
            }
        }
        None
    }

    /// Whether `name` is qualified by the schema this migration owns.
    ///
    /// Case-folded, because PostgreSQL folds an unquoted name too: `PUBLIC.t`
    /// and `public.t` are the same relation.
    fn is_schema_qualified(name: &str) -> bool {
        name.to_ascii_lowercase().starts_with("public.")
    }

    #[test]
    fn every_name_in_relation_position_is_schema_qualified() {
        // `pg_temp` is searched before `public`, so an unqualified name can be
        // shadowed by a temporary table — the hole found on #41, where it turned
        // `profile_settings_at()` into a source of fabricated parameters. Only
        // names in relation position are looked at: a column called `sex` is not
        // one, and neither is the `profile` inside `profile_id`.
        for statement in declared_string_constants() {
            for keyword in ["FROM", "JOIN", "LATERAL", "INTO", "UPDATE"] {
                for name in names_after(&statement, keyword) {
                    assert!(
                        is_schema_qualified(&name),
                        "`{name}` follows `{keyword}` unqualified in: {statement}"
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
        for statement in declared_string_constants() {
            for name in cast_target_names(&statement) {
                assert!(
                    is_schema_qualified(&name)
                        || BUILT_IN
                            .iter()
                            .any(|built_in| name.eq_ignore_ascii_case(built_in)),
                    "the cast to `{name}` is unqualified in: {statement}"
                );
            }
        }
    }

    #[test]
    fn no_statement_of_this_module_carries_an_interpolation_marker() {
        // Belt to the brace of `every_statement_of_this_module_is_a_named
        // _constant`: a constant that had been through a `format!` before
        // reaching the constructor would still show its braces here — inside a
        // quoted SQL literal included, which is exactly where an interpolated
        // value does the most damage.
        //
        // Every brace is refused, deliberately and not by oversight: the
        // PostgreSQL array literal `'{a,b}'` is legitimate SQL and is turned
        // down all the same, because telling it from a placeholder that
        // survived would take an SQL parser. A statement that needs one writes
        // `ARRAY[a, b]`, which this module reads without complaint.
        for statement in declared_string_constants() {
            assert!(
                !statement.contains('{') && !statement.contains('}'),
                "a formatting placeholder survived into: {statement}"
            );
        }
    }

    #[test]
    fn a_keyword_is_read_as_a_token_whatever_separates_it_from_its_name() {
        // The guards used to look for a keyword written with exactly one space
        // after it — and this file wraps every statement it declares, so the
        // shape they could not see was the shape in use. Measured on #6 before
        // the change: a constant naming its relation after a line break left
        // `every_name_in_relation_position_is_schema_qualified` green. The set
        // of whitespace is finite, so this one is exact.
        for (separator, shape) in [
            (" ", "one space"),
            ("  ", "two spaces"),
            ("\n    ", "a line break and its indentation"),
            ("\t", "a tabulation"),
            ("\r\n", "a carriage return and a line break"),
        ] {
            let statement = format!("SELECT 1 FROM{separator}profile");
            assert_eq!(
                names_after(&statement, "FROM"),
                vec!["profile".to_owned()],
                "the name after `FROM` is lost when {shape} separates them"
            );
        }
        assert_eq!(
            names_after("select 1 from profile", "FROM"),
            vec!["profile".to_owned()],
            "a keyword in lower case introduces a name all the same"
        );
        // `JOIN LATERAL f(…)` names `f`, and `LATERAL` picks it up on its pass.
        assert!(names_after("JOIN LATERAL public.f(x)", "JOIN").is_empty());
        assert_eq!(
            names_after("JOIN LATERAL public.f(x)", "LATERAL"),
            vec!["public.f".to_owned()]
        );
        // What is not a name is not read as one: a subquery opens with a
        // parenthesis, and the words inside a literal are text.
        assert!(names_after("SELECT 1 FROM (SELECT 2)", "FROM").is_empty());
        assert!(names_after("SELECT 'a FROM b'", "FROM").is_empty());
    }

    #[test]
    fn a_cast_is_read_in_both_spellings_postgresql_accepts() {
        // `CAST(… AS …)` carries no colons at all, so the reader that looked
        // for `::` saw no cast — measured green on #6 — and so did a `::`
        // written with spaces around it.
        for (statement, shape) in [
            ("SELECT $1::quantity_unit", "colons written tight"),
            ("SELECT $1 :: quantity_unit", "colons written spaced"),
            ("SELECT CAST($1 AS quantity_unit)", "the standard spelling"),
            (
                "SELECT CAST(\n    $1\n    AS quantity_unit\n)",
                "the standard spelling, wrapped over lines",
            ),
            (
                "SELECT cast($1 as quantity_unit)",
                "the standard spelling in lower case",
            ),
            (
                "SELECT CAST(coalesce($1, $2) AS quantity_unit)",
                "an argument of its own",
            ),
        ] {
            assert_eq!(
                cast_target_names(statement),
                vec!["quantity_unit".to_owned()],
                "the cast is not read when it is written with {shape}"
            );
        }
        // The `AS` that names an alias introduces no type, and neither does a
        // `CAST` that is only a word.
        assert!(cast_target_names("SELECT p.sex AS sex FROM public.profile AS p").is_empty());
        assert!(cast_target_names("SELECT cast_of_thousands FROM public.film").is_empty());
    }

    #[test]
    fn a_statement_written_with_escapes_reads_as_the_text_it_stands_for() {
        // A statement may be written `"… FROM\n    public.profile"` as readily
        // as across two lines, and the guards have to see the same whitespace
        // in both: left as written, `\n` is a backslash and a letter, and the
        // token after `FROM` is neither of them a name.
        assert_eq!(
            string_literals_of_declarations(r#"const A: &str = "a\tb\n\u{63}\x64\"e\\f";"#),
            vec!["a\tb\ncd\"e\\f".to_owned()]
        );
        // A raw string carries no escapes: its bytes are its content.
        assert_eq!(
            string_literals_of_declarations("const A: &str = r\"a\\tb\";"),
            vec!["a\\tb".to_owned()]
        );
    }

    #[test]
    fn the_reader_of_calls_looks_at_code_and_not_at_prose() {
        // A guard that turns red on the comment warning against the very shape
        // it forbids gets the comment deleted, not the shape. Measured on #6:
        // spelling the counter-example out in a comment turned
        // `every_statement_of_this_module_is_a_named_constant` red.
        let sqlx = "sqlx";
        let call = format!("{sqlx}::query(&format!(\"DELETE FROM {{t}}\"))");
        for (shape, source) in [
            ("a line comment", format!("// never write {call}")),
            ("a doc comment", format!("/// never write {call}")),
            ("a block comment", format!("/* never write {call} */")),
            (
                "a string",
                format!("fn f() {{ let warning = \"{call}\"; }}"),
            ),
        ] {
            assert!(
                !code_of(&source).contains(&format!("{sqlx}::")),
                "the reader of calls reads {shape} as if it were code"
            );
        }
        // And it does still read the code around them.
        assert!(code_of(&format!("// {call}\n{call}")).contains(&format!("{sqlx}::query")));
    }

    #[test]
    fn the_reader_recognises_every_form_a_declaration_can_take() {
        // What the guards below are worth is what the reader is worth: a form
        // it cannot see is a statement no guard ever reads. Each source here
        // declares the same statement in a shape that got past the
        // spelling-matched reader this replaced — measured, not supposed: a
        // constant written `r#"…"#` and naming an unqualified relation left
        // `every_name_in_relation_position_is_schema_qualified` green.
        for (form, source) in [
            ("the shape this file uses", r#"const A: &str = "SELECT 1";"#),
            ("a raw string", r##"const A: &str = r#"SELECT 1"#;"##),
            (
                "a raw string with no hash",
                r#"const A: &str = r"SELECT 1";"#,
            ),
            (
                "a raw string with two hashes",
                r###"const A: &str = r##"SELECT 1"##;"###,
            ),
            (
                "a spelled-out lifetime",
                r#"const A: &'static str = "SELECT 1";"#,
            ),
            (
                "no space around the punctuation",
                r#"const A:&str="SELECT 1";"#,
            ),
            (
                "a value wrapped to the next line",
                "const A: &str =\n    \"SELECT 1\";",
            ),
            (
                "a type alias",
                r#"type Sql = &'static str; const A: Sql = "SELECT 1";"#,
            ),
            ("a static", r#"static A: &str = "SELECT 1";"#),
            (
                "an array, whose type carries a semicolon of its own",
                r#"const A: [&str; 1] = ["SELECT 1"];"#,
            ),
            (
                "a public constant of a submodule",
                r#"mod inner { pub const A: &str = "SELECT 1"; }"#,
            ),
            (
                "a declaration behind a character literal that is a quote",
                r#"fn f() { let q = '"'; } const A: &str = "SELECT 1";"#,
            ),
            (
                "a declaration behind a block comment",
                r#"/* const A: &str = "not this one"; */ const A: &str = "SELECT 1";"#,
            ),
        ] {
            assert_eq!(
                string_literals_of_declarations(source),
                vec!["SELECT 1".to_owned()],
                "the reader does not recover a statement declared with {form}"
            );
        }
    }

    #[test]
    fn the_reader_takes_nothing_a_declaration_did_not_introduce() {
        // The other half of the same claim. A reader that simply swept up every
        // literal of the file would drag the guards over strings that are no
        // statement at all — the fixtures just above among them — and the
        // failures it invented would be answered by weakening the guards.
        for (shape, source) in [
            (
                "a local binding",
                r#"fn f() { let a = "SELECT 1 FROM profile"; }"#,
            ),
            (
                "a commented-out declaration",
                r#"// const A: &str = "SELECT 1 FROM profile";"#,
            ),
            (
                "an argument of a call",
                r#"fn f() { g("SELECT 1 FROM profile"); }"#,
            ),
            (
                "a literal after the declaration ended",
                r#"const A: &str = ""; fn f() { let b = "SELECT 1 FROM profile"; }"#,
            ),
        ] {
            assert!(
                string_literals_of_declarations(source)
                    .iter()
                    .all(String::is_empty),
                "the reader takes {shape} for a declared constant"
            );
        }
    }

    #[test]
    fn the_reader_finds_the_constants_this_file_declares() {
        // Guards the readers themselves. Every guard above iterates over
        // `declared_string_constants()`, so a parser that stopped recognising
        // the shape of a constant would return an empty list and leave three
        // tests passing over nothing.
        let declared = declared_string_constants();
        assert!(
            declared.len() >= 7,
            "only {} constant(s) found in {}: the reader is guarding nothing",
            declared.len(),
            file!()
        );
        for expected in [SELECT_PAGE, SELECT_ONE, INSERT_PROFILE, UPDATE_PROFILE] {
            assert!(
                declared.iter().any(|found| found == expected),
                "a known statement was not recovered by the reader: {expected}"
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
