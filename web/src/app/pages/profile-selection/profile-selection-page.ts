import { Component, inject } from '@angular/core';
import { CurrentProfileStore } from '../../core/profile/current-profile-store';

interface ProfileSummary {
  id: string;
  display_name: string;
}

@Component({
  selector: 'app-profile-selection-page',
  standalone: true,
  templateUrl: './profile-selection-page.html',
  styleUrl: './profile-selection-page.css',
})
export class ProfileSelectionPage {
  private readonly currentProfileStore = inject(CurrentProfileStore);

  // Données fictives pour développer l'interface avant de connecter l'API.
  readonly profiles: ProfileSummary[] = [
    { id: 'toto', display_name: 'Toto' },
    { id: 'tata', display_name: 'Tata' },
    { id: 'titi', display_name: 'Titi' },
    { id: 'tutu', display_name: 'Tutu' },
  ];

  selectProfile(profile: ProfileSummary): void {
    this.currentProfileStore.select(profile);
  }
}