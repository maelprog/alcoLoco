import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { API_BASE_URL } from './api-base-url';
import { Page, PageRequest, pageParams } from './page';

/**
 * The typed entry point to the API.
 *
 * It holds what every call shares — the base prefix, the cursor pagination
 * contract — and nothing that belongs to one endpoint: a feature declares its
 * own payload types and its own paths, and calls through here. Failures never
 * surface as `HttpErrorResponse`; the interceptor of `api-error.interceptor.ts`
 * has already turned them into `ApiError`.
 */
@Injectable({ providedIn: 'root' })
export class ApiClient {
  private readonly http = inject(HttpClient);
  private readonly baseUrl = inject(API_BASE_URL);

  /**
   * Absolute path of an API route.
   *
   * @throws Error when the path is not rooted, which would silently resolve
   * against the current page and hit whatever the router is showing.
   */
  url(path: string): string {
    if (!path.startsWith('/')) {
      throw new Error(`an API path must start with "/", got "${path}"`);
    }
    return `${this.baseUrl}${path}`;
  }

  /** `GET` on a single resource. */
  get<T>(path: string, params?: HttpParams): Observable<T> {
    return this.http.get<T>(this.url(path), { params });
  }

  /** `GET` on a paginated collection. */
  getPage<T>(path: string, request?: PageRequest): Observable<Page<T>> {
    return this.http.get<Page<T>>(this.url(path), { params: pageParams(request) });
  }

  /** `POST` of a payload, answering the created or computed resource. */
  post<T>(path: string, body: unknown): Observable<T> {
    return this.http.post<T>(this.url(path), body);
  }

  /** `PUT` of a full replacement payload. */
  put<T>(path: string, body: unknown): Observable<T> {
    return this.http.put<T>(this.url(path), body);
  }

  /** `DELETE` on a single resource. */
  delete<T>(path: string): Observable<T> {
    return this.http.delete<T>(this.url(path));
  }
}
