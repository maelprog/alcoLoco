import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { ApiClient } from '../api/api-client';
import { Page } from '../api/page';

export interface ProfileSummary {
  id: string;
  display_name: string;
}

export interface ProfileSettings {
  valid_from: string;
  weight_kg: number;
  height_cm: number;
  sex: string;
  birth_date: string;
}

export interface ProfileSettingsRequest {
  weight_kg: number;
  height_cm: number;
  sex: string;
  birth_date: string;
}

export interface UpdateProfileRequest {
  display_name: string;
  default_quantity_unit: string;
  default_ingestion_duration_seconds: number;
  default_absorption_duration_seconds: number;
  settings: ProfileSettingsRequest;
}

export interface Profile {
  id: string;
  display_name: string;
  default_quantity_unit: string;
  default_ingestion_duration_seconds: number;
  default_absorption_duration_seconds: number;
  settings: ProfileSettings;
  created_at: string;
  updated_at: string;
}

@Injectable({
  providedIn: 'root',
})
export class ProfileClient {
  private readonly api = inject(ApiClient);

  list(): Observable<Page<ProfileSummary>> {
    return this.api.getPage<ProfileSummary>('/profiles');
  }

  get(id: string): Observable<Profile> {
    return this.api.get<Profile>(`/profiles/${id}`);
  }

  update(id: string, profile: UpdateProfileRequest): Observable<Profile> {
    return this.api.put<Profile>(`/profiles/${id}`, profile);
  }
}