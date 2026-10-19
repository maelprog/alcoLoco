import { Component, OnInit, inject, signal } from '@angular/core';
import { ProfileClient, ProfileSummary } from '../../core/profile/profile-client';
import { CurrentProfileStore } from '../../core/profile/current-profile-store';

@Component({
  selector: 'app-profile-selection-page',
  standalone: true,
  templateUrl: './profile-selection-page.html',
  styleUrl: './profile-selection-page.css',
})
export class ProfileSelectionPage implements OnInit {
  private readonly profileClient = inject(ProfileClient);
  private readonly currentProfileStore = inject(CurrentProfileStore);

  readonly profiles = signal<ProfileSummary[]>([]);
  loading = true;
  errorMessage = '';

  ngOnInit(): void {
    this.profileClient.list().subscribe({
      next: (page) => {
        this.profiles.set(page.items);
        this.loading = false;
      },
      error: () => {
        this.errorMessage = 'Impossible de charger les profils. Réessaie plus tard.';
        this.loading = false;
      },
    });
  }

  selectProfile(profile: ProfileSummary): void {
    this.currentProfileStore.select(profile);
  }
}