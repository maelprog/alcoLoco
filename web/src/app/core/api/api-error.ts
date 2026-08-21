import { HttpErrorResponse } from '@angular/common/http';

import { FieldError, ProblemDetails, ProblemType, isProblemDetails } from './problem-details';

/**
 * A failed API call, in the single shape the rest of the front deals with.
 *
 * Every failure — a problem document from a handler, or a transport failure
 * that never reached one — arrives here, so a caller has one type to catch and
 * one place to read `type` from.
 */
export class ApiError extends Error {
  constructor(readonly problem: ProblemDetails) {
    super(`${problem.status} ${problem.type}: ${problem.detail}`);
    this.name = 'ApiError';
  }

  /** Stable identifier of the problem kind. What callers branch on. */
  get type(): string {
    return this.problem.type;
  }

  /** HTTP status of the failure; `0` when no response was received. */
  get status(): number {
    return this.problem.status;
  }

  /** Field-level failures, empty unless the API reported a validation. */
  get fieldErrors(): readonly FieldError[] {
    return this.problem.errors;
  }
}

/**
 * Wording shown to the user, keyed by problem `type`.
 *
 * The API answers in English and the front owns the displayed text, so nothing
 * of `title` or `detail` is ever rendered. Keying on `type` is what makes that
 * possible: it is the stable member, `title` is not.
 */
const USER_MESSAGES: Record<string, string> = {
  [ProblemType.badRequest]: 'La requête envoyée est mal formée.',
  [ProblemType.notFound]: 'Cette donnée est introuvable.',
  [ProblemType.methodNotAllowed]: 'Cette action n’est pas disponible ici.',
  [ProblemType.conflict]: 'L’opération entre en conflit avec l’état actuel des données.',
  [ProblemType.validationFailed]: 'Le formulaire contient des valeurs invalides.',
  [ProblemType.internalError]: 'Le serveur a rencontré une erreur. Réessayez dans un instant.',
};

/** Shown when the request never reached the API. */
const UNREACHABLE_MESSAGE = 'Le serveur est injoignable. Vérifiez votre connexion.';

/** Shown for a failure the front does not know about yet. */
const FALLBACK_MESSAGE = 'Une erreur inattendue est survenue.';

/**
 * The French sentence to show the user for a failure.
 *
 * An unknown `type` falls back to a generic sentence rather than to the API
 * `title`: the API writes English for machines and logs, not for readers.
 */
export function userMessage(error: ApiError): string {
  const known = USER_MESSAGES[error.type];
  if (known !== undefined) {
    return known;
  }
  return error.status === 0 ? UNREACHABLE_MESSAGE : FALLBACK_MESSAGE;
}

/**
 * Turns an HTTP failure into an {@link ApiError}.
 *
 * A body that is not a problem document means the failure never reached a
 * handler — no proxy, no network, an HTML error page — and is reported under
 * the `about:blank` type RFC 7807 §4.2 reserves for "no problem type".
 */
export function toApiError(response: HttpErrorResponse): ApiError {
  const body: unknown = response.error;
  if (isProblemDetails(body)) {
    return new ApiError({ ...body, errors: body.errors ?? [] });
  }
  return new ApiError({
    type: ProblemType.unknown,
    title: response.statusText,
    status: response.status,
    detail: response.message,
    errors: [],
  });
}
