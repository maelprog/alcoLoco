import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { ApiClient } from '../api/api-client';
import { HEALTH_PATH, HealthResponse } from './health';

/** Reads `GET /health`. */
@Injectable({ providedIn: 'root' })
export class HealthClient {
  private readonly api = inject(ApiClient);

  /** Asks the API for the state of the process and of its database. */
  check(): Observable<HealthResponse> {
    return this.api.get<HealthResponse>(HEALTH_PATH);
  }
}
