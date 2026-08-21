import { Component } from '@angular/core';
import { RouterLink } from '@angular/router';

/** Answer to a URL no route claims, mirroring the API’s `not_found` problem. */
@Component({
  selector: 'app-not-found-page',
  imports: [RouterLink],
  template: `
    <h2>Page introuvable</h2>
    <p>Cette adresse ne correspond à aucun écran de l’application.</p>
    <a routerLink="/profiles">Revenir au choix du profil</a>
  `,
})
export class NotFoundPage {}
