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

  readonly weightKg = signal<number | string>('');
  readonly heightCm = signal<number | string>('');
  readonly sex = signal('');
  readonly birthDate = signal('');

  readonly defaultUnit = signal('cl');
  readonly defaultIngestionDuration = signal(15);
  readonly defaultAbsorptionDuration = signal(30);

  readonly invalidFields = signal<string[]>([]);

  readonly errorMessage = signal('');
  readonly successMessage = signal('');
  readonly loading = signal(true);
  readonly saving = signal(false);

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

  clearFieldError(field: string): void {
    this.invalidFields.update((fields) =>
      fields.filter((invalidField) => invalidField !== field),
    );
  }

  saveProfile(): void {
    if (this.saving()) {
      return;
    }
    this.errorMessage.set('');
    this.successMessage.set('');
    this.invalidFields.set([]);

    const currentProfile = this.currentProfileStore.profile();

    if (!currentProfile) {
      this.errorMessage.set('Aucun profil sélectionné.');
      return;
    }

    const weightKg = Number(this.weightKg());
    const heightCm = Number(this.heightCm());
    const ingestionDuration = Number(this.defaultIngestionDuration());
    const absorptionDuration = Number(this.defaultAbsorptionDuration());

    const invalidFields: string[] = [];

    // Vérification du poids
    if (
      this.weightKg() === '' ||
      !Number.isFinite(weightKg) ||
      weightKg <= 0
    ) {
      invalidFields.push('weightKg');
    }

    // Vérification de la taille
    if (
      this.heightCm() === '' ||
      !Number.isFinite(heightCm) ||
      heightCm <= 0
    ) {
      invalidFields.push('heightCm');
    }

    // Vérification du sexe
    if (!['female', 'male'].includes(this.sex())) {
      invalidFields.push('sex');
    }

    // Vérification de la date de naissance
    const birthDate = this.birthDate();
    const parsedBirthDate = new Date(`${birthDate}T00:00:00`);
    const today = new Date();
    today.setHours(0, 0, 0, 0);

    if (
      !birthDate ||
      Number.isNaN(parsedBirthDate.getTime()) ||
      parsedBirthDate > today
    ) {
      invalidFields.push('birthDate');
    }

    // Vérification de la durée d'ingestion
    if (
      !Number.isInteger(ingestionDuration) ||
      ingestionDuration < 0 ||
      ingestionDuration % 5 !== 0
    ) {
      invalidFields.push('ingestionDuration');
    }

    // Vérification de la durée d'absorption
    if (
      !Number.isInteger(absorptionDuration) ||
      absorptionDuration < 0 ||
      absorptionDuration % 5 !== 0
    ) {
      invalidFields.push('absorptionDuration');
    }

    // Si des champs sont invalides, on les marque en rouge.
    if (invalidFields.length > 0) {
      this.invalidFields.set(invalidFields);
      this.errorMessage.set(
        'Certaines valeurs saisies sont invalides. Vérifie les champs en rouge.',
      );
      return;
    }

    // Enregistrement auprès de l'API
    this.saving.set(true);
    this.profileClient.update(currentProfile.id, {
      display_name: currentProfile.display_name,
      default_quantity_unit: this.defaultUnit(),
      default_ingestion_duration_seconds: ingestionDuration * 60,
      default_absorption_duration_seconds: absorptionDuration * 60,
      settings: {
        weight_kg: weightKg,
        height_cm: heightCm,
        sex: this.sex(),
        birth_date: birthDate,
      },
    }).subscribe({
      next: () => {
        this.successMessage.set(
          'Les modifications ont bien été enregistrées.',
        );
        this.invalidFields.set([]);
        this.saving.set(false);
      },
      error: () => {
        this.errorMessage.set(
          'Impossible d’enregistrer les modifications. Vérifie les valeurs saisies.',
        );
        this.saving.set(false);
      },
    });
  }
}