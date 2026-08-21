import { InjectionToken } from '@angular/core';

/**
 * Prefix every API call is built on.
 *
 * The API serves its routes at the root (`GET /health`), so the prefix is
 * stripped before a request reaches it: in development by the dev server proxy
 * declared in `proxy.conf.json`, in production by whatever reverse proxy fronts
 * both the static bundle and the API. Keeping the browser on a single origin is
 * what spares the API the CORS layer it does not have.
 */
export const API_BASE_URL = new InjectionToken<string>('API_BASE_URL', {
  providedIn: 'root',
  factory: () => '/api',
});
