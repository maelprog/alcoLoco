/**
 * An instant as the API spells it: ISO 8601, UTC, `Z` suffix.
 *
 * No local date ever crosses the API (SPEC.md §7): converting to the reader’s
 * time zone is the front’s job, done at display time by `DatePipe`, never by
 * rewriting the value carried in a payload.
 */
export type Timestamp = string;
