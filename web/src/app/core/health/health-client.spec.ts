import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';

import { HealthResponse } from './health';
import { HealthClient } from './health-client';

describe('HealthClient', () => {
  let client: HealthClient;
  let http: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    client = TestBed.inject(HealthClient);
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => {
    http.verify();
  });

  it('reads GET /health behind the API prefix', () => {
    client.check().subscribe();

    const request = http.expectOne('/api/health');
    expect(request.request.method).toBe('GET');
    request.flush({ status: 'ok', database: 'up', checked_at: '2026-08-20T21:04:05Z' });
  });

  it('reads back the payload the API documents, member by member', () => {
    let report: HealthResponse | undefined;
    client.check().subscribe((received) => {
      report = received;
    });

    http.expectOne('/api/health').flush({
      status: 'degraded',
      database: 'down',
      checked_at: '2026-08-20T21:04:05Z',
    });

    expect(report).toEqual({
      status: 'degraded',
      database: 'down',
      checked_at: '2026-08-20T21:04:05Z',
    });
  });
});
