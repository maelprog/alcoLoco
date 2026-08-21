import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';

import { API_BASE_URL } from './api-base-url';
import { ApiClient } from './api-client';
import { MAX_PAGE_LIMIT, Page } from './page';

interface Item {
  id: string;
}

describe('ApiClient', () => {
  let client: ApiClient;
  let http: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    client = TestBed.inject(ApiClient);
    http = TestBed.inject(HttpTestingController);
  });

  afterEach(() => {
    http.verify();
  });

  it('prefixes every call with the configured base URL', () => {
    client.get('/profiles').subscribe();

    http.expectOne('/api/profiles').flush({});
  });

  it('refuses a path that is not rooted', () => {
    // A relative path would resolve against whatever the router is showing, so
    // the same call would hit a different URL depending on the current screen.
    expect(() => client.url('profiles')).toThrow();
  });

  it('sends each verb on the addressed resource', () => {
    client.post('/events', { name: 'Soirée' }).subscribe();
    const created = http.expectOne('/api/events');
    expect(created.request.method).toBe('POST');
    expect(created.request.body).toEqual({ name: 'Soirée' });
    created.flush({});

    client.put('/events/1', { name: 'Soirée' }).subscribe();
    const replaced = http.expectOne('/api/events/1');
    expect(replaced.request.method).toBe('PUT');
    replaced.flush({});

    client.delete('/events/1').subscribe();
    const removed = http.expectOne('/api/events/1');
    expect(removed.request.method).toBe('DELETE');
    removed.flush({});
  });

  describe('cursor pagination', () => {
    it('asks for a page without parameters when none is given', () => {
      client.getPage<Item>('/drinks').subscribe();

      const request = http.expectOne((candidate) => candidate.url === '/api/drinks');
      expect(request.request.params.keys()).toEqual([]);
      request.flush({ items: [], next_cursor: null });
    });

    it('passes the limit and the cursor as the API spells them', () => {
      client.getPage<Item>('/drinks', { limit: 10, cursor: 'abc' }).subscribe();

      const request = http.expectOne((candidate) => candidate.url === '/api/drinks');
      expect(request.request.params.get('limit')).toBe('10');
      expect(request.request.params.get('cursor')).toBe('abc');
      request.flush({ items: [], next_cursor: null });
    });

    it('reads back the page shape the API answers', () => {
      let page: Page<Item> | undefined;
      client.getPage<Item>('/drinks').subscribe((received) => {
        page = received;
      });

      http.expectOne('/api/drinks').flush({ items: [{ id: 'one' }], next_cursor: '0199-cursor' });

      expect(page?.items).toEqual([{ id: 'one' }]);
      expect(page?.next_cursor).toBe('0199-cursor');
    });

    it('refuses a limit the API would reject, without spending a round trip', () => {
      expect(() => client.getPage<Item>('/drinks', { limit: MAX_PAGE_LIMIT + 1 })).toThrow(
        RangeError,
      );
      expect(() => client.getPage<Item>('/drinks', { limit: 0 })).toThrow(RangeError);
      expect(() => client.getPage<Item>('/drinks', { limit: 1.5 })).toThrow(RangeError);
    });
  });
});

describe('ApiClient with another base URL', () => {
  it('builds its calls on the URL it was given', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        { provide: API_BASE_URL, useValue: 'https://api.example.test' },
      ],
    });
    const client = TestBed.inject(ApiClient);
    const http = TestBed.inject(HttpTestingController);

    client.get('/profiles').subscribe();

    http.expectOne('https://api.example.test/profiles').flush({});
    http.verify();
  });
});
