import { HttpErrorResponse } from '@angular/common/http';

import { ApiError, toApiError, userMessage } from './api-error';
import { ProblemDetails, ProblemType, isProblemDetails } from './problem-details';

function problem(overrides: Partial<ProblemDetails> = {}): ProblemDetails {
  return {
    type: ProblemType.notFound,
    title: 'Resource not found',
    status: 404,
    detail: 'no resource at /profiles/1',
    errors: [],
    ...overrides,
  };
}

describe('toApiError', () => {
  it('keeps the problem document the API sent', () => {
    const body = problem({
      type: ProblemType.validationFailed,
      status: 400,
      errors: [{ field: 'limit', detail: 'must be between 1 and 200' }],
    });

    const error = toApiError(new HttpErrorResponse({ status: 400, error: body }));

    expect(error).toBeInstanceOf(ApiError);
    expect(error.type).toBe(ProblemType.validationFailed);
    expect(error.status).toBe(400);
    expect(error.fieldErrors).toEqual([{ field: 'limit', detail: 'must be between 1 and 200' }]);
  });

  it('reports a body that is not a problem document as an untyped failure', () => {
    // A proxy answering HTML, for instance: no handler was ever reached, so
    // there is no problem type to speak of (RFC 7807 §4.2).
    const error = toApiError(
      new HttpErrorResponse({ status: 502, statusText: 'Bad Gateway', error: '<html>oops</html>' }),
    );

    expect(error.type).toBe(ProblemType.unknown);
    expect(error.status).toBe(502);
    expect(error.fieldErrors).toEqual([]);
  });

  it('reports a transport failure as status 0', () => {
    const error = toApiError(
      new HttpErrorResponse({ status: 0, error: new ProgressEvent('error') }),
    );

    expect(error.type).toBe(ProblemType.unknown);
    expect(error.status).toBe(0);
  });
});

describe('isProblemDetails', () => {
  it('rejects payloads that miss a required member', () => {
    expect(isProblemDetails(problem())).toBe(true);
    expect(isProblemDetails({ ...problem(), type: undefined })).toBe(false);
    expect(isProblemDetails({ ...problem(), status: '404' })).toBe(false);
    expect(isProblemDetails(null)).toBe(false);
    expect(isProblemDetails('not found')).toBe(false);
  });
});

describe('userMessage', () => {
  it('reads the stable `type` and never the `title`', () => {
    // The API may reword `title` at will — it is English, written for logs. The
    // sentence shown to the user is chosen on `type` alone; a lookup keyed on
    // `title` would fall through here and return the generic sentence.
    const error = new ApiError(
      problem({ type: ProblemType.notFound, title: 'Totally reworded by the backend' }),
    );

    expect(userMessage(error)).toBe('Cette donnée est introuvable.');
  });

  it('never shows the English wording of the API', () => {
    const error = new ApiError(
      problem({ type: ProblemType.internalError, status: 500, detail: 'boom in handler #3' }),
    );
    const message = userMessage(error);

    expect(message).not.toContain('Resource not found');
    expect(message).not.toContain('boom in handler #3');
    expect(message).toBe('Le serveur a rencontré une erreur. Réessayez dans un instant.');
  });

  it('gives every problem kind the API mints its own French sentence', () => {
    const kinds = [
      ProblemType.badRequest,
      ProblemType.notFound,
      ProblemType.methodNotAllowed,
      ProblemType.conflict,
      ProblemType.validationFailed,
      ProblemType.internalError,
    ];
    const messages = kinds.map((type) => userMessage(new ApiError(problem({ type }))));

    expect(new Set(messages).size).toBe(kinds.length);
    expect(messages).not.toContain('Une erreur inattendue est survenue.');
  });

  it('distinguishes an unreachable server from an unknown problem kind', () => {
    expect(userMessage(new ApiError(problem({ type: ProblemType.unknown, status: 0 })))).toBe(
      'Le serveur est injoignable. Vérifiez votre connexion.',
    );
    expect(
      userMessage(new ApiError(problem({ type: '/problems/invented_tomorrow', status: 418 }))),
    ).toBe('Une erreur inattendue est survenue.');
  });
});
