import { Routes } from '@angular/router';

/**
 * The screens of the application (SPEC.md §5).
 *
 * Every screen is lazily loaded: a route added later brings its own chunk and
 * the initial bundle stays the shell. Paths are spelled in English, like every
 * other identifier of the code base; the displayed titles are in French, like
 * every other string the user reads.
 */
export const routes: Routes = [
  {
    path: '',
    pathMatch: 'full',
    redirectTo: 'profiles',
  },
  {
    path: 'profiles',
    title: 'Choix du profil — alcoLoco',
    loadComponent: () =>
      import('./features/profile-selection/profile-selection-page').then(
        (m) => m.ProfileSelectionPage,
      ),
  },
  {
    path: 'profile',
    title: 'Mon profil — alcoLoco',
    loadComponent: () => import('./features/profile/profile-page').then((m) => m.ProfilePage),
  },
  {
    path: 'events',
    title: 'Événements — alcoLoco',
    loadComponent: () => import('./features/events/event-list-page').then((m) => m.EventListPage),
  },
  {
    path: 'events/:eventId',
    title: 'Événement — alcoLoco',
    loadComponent: () =>
      import('./features/events/event-detail-page').then((m) => m.EventDetailPage),
  },
  {
    path: '**',
    title: 'Page introuvable — alcoLoco',
    loadComponent: () => import('./features/not-found/not-found-page').then((m) => m.NotFoundPage),
  },
];
