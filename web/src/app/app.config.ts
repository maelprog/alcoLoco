import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { apiErrorInterceptor } from './core/api/api-error.interceptor';
import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    // Path parameters reach a screen as component inputs, so no screen has to
    // read `ActivatedRoute` to learn which resource it is showing.
    provideRouter(routes, withComponentInputBinding()),
    // Every failure leaves the HTTP layer as an `ApiError`, never as a raw
    // `HttpErrorResponse`.
    provideHttpClient(withInterceptors([apiErrorInterceptor])),
  ],
};
