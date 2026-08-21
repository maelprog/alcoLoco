import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';

import { apiErrorInterceptor } from '../../core/api/api-error.interceptor';
import { ProblemType } from '../../core/api/problem-details';
import { HealthIndicator } from './health-indicator';

/** The rendered text, with every kind of whitespace collapsed to one space. */
function text(fixture: ComponentFixture<HealthIndicator>): string {
  return ((fixture.nativeElement as HTMLElement).textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('HealthIndicator', () => {
  let http: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(withInterceptors([apiErrorInterceptor])),
        provideHttpClientTesting(),
      ],
    });
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => {
    http.verify();
  });

  it('calls GET /health as soon as it is shown', () => {
    TestBed.createComponent(HealthIndicator);

    expect(http.expectOne('/api/health').request.method).toBe('GET');
  });

  it('shows a healthy API and its database', async () => {
    const fixture = TestBed.createComponent(HealthIndicator);
    http
      .expectOne('/api/health')
      .flush({ status: 'ok', database: 'up', checked_at: '2026-08-20T21:04:05Z' });
    await fixture.whenStable();

    expect(text(fixture)).toContain('API : en ligne');
    expect(text(fixture)).toContain('base : connectée');
    expect(fixture.nativeElement.querySelector('.health').dataset.state).toBe('ok');
  });

  it('shows a degraded API when the database does not answer', async () => {
    const fixture = TestBed.createComponent(HealthIndicator);
    http
      .expectOne('/api/health')
      .flush({ status: 'degraded', database: 'down', checked_at: '2026-08-20T21:04:05Z' });
    await fixture.whenStable();

    expect(text(fixture)).toContain('API : dégradée');
    expect(text(fixture)).toContain('base : injoignable');
    expect(fixture.nativeElement.querySelector('.health').dataset.state).toBe('degraded');
  });

  it('renders the probe instant in the reader’s time zone, never the raw UTC string', async () => {
    const fixture = TestBed.createComponent(HealthIndicator);
    http
      .expectOne('/api/health')
      .flush({ status: 'ok', database: 'up', checked_at: '2026-08-20T21:04:05Z' });
    await fixture.whenStable();

    const at = new Date('2026-08-20T21:04:05Z');
    const local = [at.getHours(), at.getMinutes(), at.getSeconds()]
      .map((part) => String(part).padStart(2, '0'))
      .join(':');

    expect(text(fixture)).toContain(local);
    expect(text(fixture)).not.toContain('2026-08-20T21:04:05Z');
  });

  it('shows the French wording of an API failure, not the English one', async () => {
    const fixture = TestBed.createComponent(HealthIndicator);
    http.expectOne('/api/health').flush(
      {
        type: ProblemType.internalError,
        title: 'Internal server error',
        status: 500,
        detail: 'the request could not be processed',
        errors: [],
      },
      { status: 500, statusText: 'Internal Server Error' },
    );
    await fixture.whenStable();

    expect(text(fixture)).toContain('Le serveur a rencontré une erreur');
    expect(text(fixture)).not.toContain('Internal server error');
    expect(fixture.nativeElement.querySelector('.health').dataset.state).toBe('unreachable');
  });

  it('says the server is unreachable when the call does not even leave', async () => {
    const fixture = TestBed.createComponent(HealthIndicator);
    http.expectOne('/api/health').error(new ProgressEvent('error'));
    await fixture.whenStable();

    expect(text(fixture)).toContain('Le serveur est injoignable');
  });
});
