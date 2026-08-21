import { Component } from '@angular/core';

/**
 * The current profile: its parameters, its input preferences, its drink
 * history and its own alcohol level outside any event (SPEC.md §5.1, §5.5).
 */
@Component({
  selector: 'app-profile-page',
  template: `
    <h2>Mon profil</h2>
    <p>Paramètres, préférences de saisie et historique des consommations.</p>
  `,
})
export class ProfilePage {}
