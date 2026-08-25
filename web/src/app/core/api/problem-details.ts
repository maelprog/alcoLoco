/**
 * The error payload of the API, as posed by the backend (RFC 7807
 * `application/problem+json`).
 *
 * The member names are the ones on the wire and are therefore `snake_case`
 * where the backend spells them so.
 */

/** One field-level validation failure, as carried by `errors[]`. */
export interface FieldError {
  /** Name of the offending member, spelled as in the request payload. */
  field: string;
  /** Explanation of what is wrong with it. Written in English by the API. */
  detail: string;
}

/** Body of every API failure. */
export interface ProblemDetails {
  /** Stable identifier of the problem kind, e.g. `/problems/not_found`. */
  type: string;
  /** Short summary of the kind. English, and never shown to the user. */
  title: string;
  /** HTTP status, repeated in the body as RFC 7807 prescribes. */
  status: number;
  /** Explanation specific to this occurrence. English, for logs and support. */
  detail: string;
  /** Field-level failures; empty for every problem that is not a validation. */
  errors: FieldError[];
}

/**
 * The problem identifiers the API mints, mirrored from `ApiError::slug` in
 * `crates/api/src/error.rs`.
 *
 * `type` is the stable contract: it is what the front branches on. `title` is
 * an English summary that may be reworded at any time, so nothing here — no
 * message lookup, no test, no `switch` — may key on it.
 *
 * The mirroring is checked, in both directions, by
 * `crates/api/tests/front_contract.rs`: it reads this object and `error.rs` and
 * compares them, so a value rewritten here and a seventh kind added there both
 * fail the Rust gate. Nothing in `web/` can catch either — a test living here
 * would be reading these very constants. Keep this object a plain literal of
 * string literals, which is the shape that test parses.
 */
export const ProblemType = {
  badRequest: '/problems/bad_request',
  notFound: '/problems/not_found',
  methodNotAllowed: '/problems/method_not_allowed',
  conflict: '/problems/conflict',
  validationFailed: '/problems/validation_failed',
  internalError: '/problems/internal_error',
  /**
   * No problem type — RFC 7807 §4.2. Used when the failure never reached a
   * handler, so no problem document was ever produced: the network is down,
   * a proxy answered HTML, the request was cancelled.
   */
  unknown: 'about:blank',
} as const;

/** One of the identifiers of {@link ProblemType}. */
export type ProblemTypeIdentifier = (typeof ProblemType)[keyof typeof ProblemType];

/** Media type of an API error payload. */
export const PROBLEM_JSON = 'application/problem+json';

/** Tells whether an error body is a problem document the API produced. */
export function isProblemDetails(body: unknown): body is ProblemDetails {
  if (typeof body !== 'object' || body === null) {
    return false;
  }
  const candidate = body as Partial<ProblemDetails>;
  return (
    typeof candidate.type === 'string' &&
    typeof candidate.title === 'string' &&
    typeof candidate.status === 'number' &&
    typeof candidate.detail === 'string'
  );
}
