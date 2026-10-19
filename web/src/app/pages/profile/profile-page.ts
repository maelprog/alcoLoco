import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { CurrentProfileStore } from '../../core/profile/current-profile-store';
import { ProfileClient } from '../../core/profile/profile-client';

@Component({
  selector: 'app-profile-page',
  standalone: true,
  imports: [FormsModule],
  templateUrl: './profile-page.html',
  styleUrl: './profile-page.css',
})
export class ProfilePage implements OnInit {
  private readonly profileClient = inject(ProfileClient);
  private readonly currentProfileStore = inject(CurrentProfileStore);

  readonly weightKg = signal('');
  readonly heightCm = signal('');
  readonly sex = signal('');
  readonly birthDate = signal('');

  readonly defaultUnit = signal('cl');
  readonly defaultIngestionDuration = signal(15);
  readonly defaultAbsorptionDuration = signal(30);

  readonly errorMessage = signal('');
  readonly successMessage = signal('');
  readonly loading = signal(true);

  ngOnInit(): void {
    const profile = this.currentProfileStore.profile();

    if (!profile) {
      this.errorMessage.set('Aucun profil sélectionné.');
      this.loading.set(false);
      return;
    }

    this.profileClient.get(profile.id).subscribe({
      next: (data) => {
        this.weightKg.set(String(data.settings.weight_kg));
        this.heightCm.set(String(data.settings.height_cm));
        this.sex.set(data.settings.sex);
        this.birthDate.set(data.settings.birth_date);

        this.defaultUnit.set(data.default_quantity_unit);
        this.defaultIngestionDuration.set(
          data.default_ingestion_duration_seconds / 60,
        );
        this.defaultAbsorptionDuration.set(
          data.default_absorption_duration_seconds / 60,
        );
        this.loading.set(false);
      },
      error: () => {
        this.errorMessage.set(
          'Impossible de charger les données du profil.',
        );
        this.loading.set(false);
      },
    });
  }

  saveProfile(): void {
    this.errorMessage.set('');
    this.successMessage.set('');

    const currentProfile = this.currentProfileStore.profile();

    if (!currentProfile) {
      this.errorMessage.set('Aucun profil sélectionné.');
      return;
    }

    const weightKg = Number(this.weightKg());
    const heightCm = Number(this.heightCm());
    const ingestionDuration = Number(this.defaultIngestionDuration());
    const absorptionDuration = Number(this.defaultAbsorptionDuration());

    if (
      this.weightKg() === '' ||
      this.heightCm() === '' ||
      !this.sex() ||
      !this.birthDate()
    ) {
      this.errorMessage.set(
        'Veuillez renseigner tous les champs physiologiques obligatoires.',
      );
      return;
    }

    if (
      !Number.isFinite(weightKg) || weightKg <= 0 ||
      !Number.isFinite(heightCm) || heightCm <= 0 ||
      !Number.isInteger(ingestionDuration) || ingestionDuration < 0 ||
      !Number.isInteger(absorptionDuration) || absorptionDuration < 0
    ) {
      this.errorMessage.set('Certaines valeurs saisies sont invalides.');
      return;
    }

    this.profileClient.update(currentProfile.id, {
      display_name: currentProfile.display_name,
      default_quantity_unit: this.defaultUnit(),
      default_ingestion_duration_seconds: ingestionDuration * 60,
      default_absorption_duration_seconds: absorptionDuration * 60,
      settings: {
        weight_kg: weightKg,
        height_cm: heightCm,
        sex: this.sex(),
        birth_date: this.birthDate(),
      },
    }).subscribe({
      next: () => {
        this.successMessage.set('Les modifications ont bien été enregistrées.');
      },
      error: () => {
        this.errorMessage.set(
          'Impossible d’enregistrer les modifications. Vérifie les valeurs saisies.',
        );
      },
    });
  }
}