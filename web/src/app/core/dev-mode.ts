import { InjectionToken, isDevMode } from '@angular/core';

/**
 * Whether the application runs in development.
 *
 * A token rather than a direct `isDevMode()` call at the use site: a test can
 * then render both the development and the production shell without rebuilding
 * the bundle, which is the only way "shown in dev, hidden in production" can be
 * asserted at all.
 */
export const DEV_MODE = new InjectionToken<boolean>('DEV_MODE', {
  providedIn: 'root',
  factory: () => isDevMode(),
});
