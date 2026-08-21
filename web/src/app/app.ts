import { Component, inject, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { DEV_MODE } from './core/dev-mode';
import { CurrentProfileStore } from './core/profile/current-profile-store';
import { HealthIndicator } from './layout/health-indicator/health-indicator';
import { NAV_LINKS } from './layout/nav-links';

/**
 * The application shell: title, navigation, page container, and — in
 * development only — the `GET /health` banner.
 */
@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, HealthIndicator],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App {
  private readonly currentProfile = inject(CurrentProfileStore);

  protected readonly title = signal('alcoLoco');

  /** The main navigation, checked against the routing table by its own test. */
  protected readonly navLinks = NAV_LINKS;

  /** The profile in use, shown in the header. `null` until one is picked. */
  protected readonly profile = this.currentProfile.profile;

  /** The health banner is a development aid and never ships to a user. */
  protected readonly devMode = inject(DEV_MODE);
}
