import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { ApiClient } from '../api/api-client';
import { Page } from '../api/page';

export interface ProfileSummary {
  id: string;
  display_name: string;
}

@Injectable({
  providedIn: 'root',
})
export class ProfileClient {
  private readonly api = inject(ApiClient);

  list(): Observable<Page<ProfileSummary>> {
    return this.api.getPage<ProfileSummary>('/profiles');
  }
}