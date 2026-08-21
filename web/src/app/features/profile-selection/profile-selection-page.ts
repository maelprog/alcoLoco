import { Component } from '@angular/core';

/**
 * Landing screen: the user picks a profile among the predefined ones
 * (SPEC.md §5.1). The list itself arrives with #8, which reads `GET /profiles`.
 */
@Component({
  selector: 'app-profile-selection-page',
  template: `
    <h2>Choix du profil</h2>
    <p>Sélectionnez le profil à utiliser. La liste des profils sera branchée avec l’écran dédié.</p>
  `,
})
export class ProfileSelectionPage {}
