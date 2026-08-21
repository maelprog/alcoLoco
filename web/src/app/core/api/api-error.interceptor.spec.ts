import { HttpClient, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';

import { ApiError } from './api-error';
import { apiErrorInterceptor } from './api-error.interceptor';
import { PROBLEM_JSON, ProblemType } from './problem-details';

describe('apiErrorInterceptor', () => {
  let http: HttpClient;
  let requests: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(withInterceptors([apiErrorInterceptor])),
        provideHttpClientTesting(),
      ],
    });
    http = TestBed.inject(HttpClient);
    requests = TestBed.inject(HttpTestingController);
  });

  afterEach(() => {
    requests.verify();
  });

  it('hands the caller an ApiError instead of an HttpErrorResponse', async () => {
    const answered = new Promise<unknown>((resolve) => {
      http.get('/api/profiles/1').subscribe({ error: resolve });
    });

    requests.expectOne('/api/profiles/1').flush(
      {
        type: ProblemType.notFound,
        title: 'Resource not found',
        status: 404,
        detail: 'no resource at /profiles/1',
        errors: [],
      },
      { status: 404, statusText: 'Not Found', headers: { 'Content-Type': PROBLEM_JSON } },
    );

    const error = await answered;
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).type).toBe(ProblemType.notFound);
    expect((error as ApiError).status).toBe(404);
  });

  it('turns a network failure into an untyped ApiError rather than letting it through raw', async () => {
    const answered = new Promise<unknown>((resolve) => {
      http.get('/api/health').subscribe({ error: resolve });
    });

    requests.expectOne('/api/health').error(new ProgressEvent('error'));

    const error = await answered;
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).type).toBe(ProblemType.unknown);
  });

  it('leaves a successful answer untouched', async () => {
    const answered = new Promise<unknown>((resolve) => {
      http.get('/api/health').subscribe({ next: resolve });
    });

    requests.expectOne('/api/health').flush({ status: 'ok' });

    expect(await answered).toEqual({ status: 'ok' });
  });
});
