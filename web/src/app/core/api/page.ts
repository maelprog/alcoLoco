import { HttpParams } from '@angular/common/http';

/**
 * Cursor pagination, mirrored from `crates/api/src/pagination.rs`.
 *
 * A request carries `?limit=&cursor=`, an answer carries
 * `{ "items": [...], "next_cursor": <string|null> }`. There is no offset and no
 * total: the cursor is the identifier of the last item of the previous page.
 */

/** Page size the API uses when the request does not ask for one. */
export const DEFAULT_PAGE_LIMIT = 50;

/** Largest page size the API will serve. */
export const MAX_PAGE_LIMIT = 200;

/** One page of a collection. */
export interface Page<T> {
  /** The items of this page, in the API’s order. */
  items: T[];
  /** Cursor to pass to get the next page, `null` on the last one. */
  next_cursor: string | null;
}

/** What the caller asks for. Both members are optional. */
export interface PageRequest {
  /** Number of items wanted, within `1..=MAX_PAGE_LIMIT`. */
  limit?: number;
  /** Cursor returned as `next_cursor` by the previous page. */
  cursor?: string | null;
}

/**
 * Builds the query string of a page request.
 *
 * A limit outside the bounds throws instead of being clamped or sent: the API
 * would answer `400` with a validation problem, and a caller that asks for 500
 * items has a bug the round trip would only hide.
 *
 * @throws RangeError when `limit` is not a whole number within the bounds.
 */
export function pageParams(request: PageRequest = {}): HttpParams {
  let params = new HttpParams();

  if (request.limit !== undefined) {
    if (!Number.isInteger(request.limit) || request.limit < 1 || request.limit > MAX_PAGE_LIMIT) {
      throw new RangeError(
        `limit must be a whole number between 1 and ${MAX_PAGE_LIMIT}, got ${request.limit}`,
      );
    }
    params = params.set('limit', request.limit);
  }

  if (request.cursor !== undefined && request.cursor !== null && request.cursor !== '') {
    params = params.set('cursor', request.cursor);
  }

  return params;
}
