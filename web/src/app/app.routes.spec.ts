import { provideLocationMocks } from '@angular/common/testing';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter, withComponentInputBinding } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { routes } from './app.routes';
import { EventDetailPage } from './features/events/event-detail-page';
import { EventListPage } from './features/events/event-list-page';
import { NotFoundPage } from './features/not-found/not-found-page';
import { ProfilePage } from './features/profile/profile-page';
import { ProfileSelectionPage } from './features/profile-selection/profile-selection-page';
import { NAV_LINKS } from './layout/nav-links';

describe('application routes', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes, withComponentInputBinding()), provideLocationMocks()],
    });
  });

  it('sends the bare URL to the profile selection', async () => {
    const harness = await RouterTestingHarness.create();

    await harness.navigateByUrl('/', ProfileSelectionPage);

    expect(TestBed.inject(Router).url).toBe('/profiles');
  });

  it('navigates to every main screen', async () => {
    const harness = await RouterTestingHarness.create();

    await harness.navigateByUrl('/profiles', ProfileSelectionPage);
    expect(harness.routeNativeElement?.textContent).toContain('Choix du profil');

    await harness.navigateByUrl('/profile', ProfilePage);
    expect(harness.routeNativeElement?.textContent).toContain('Mon profil');

    await harness.navigateByUrl('/events', EventListPage);
    expect(harness.routeNativeElement?.textContent).toContain('Événements');
  });

  it('hands the event screen the identifier from the URL', async () => {
    const harness = await RouterTestingHarness.create();

    const screen = await harness.navigateByUrl(
      '/events/01920000-0000-7000-8000-000000000001',
      EventDetailPage,
    );

    expect(screen.eventId()).toBe('01920000-0000-7000-8000-000000000001');
    expect(harness.routeNativeElement?.textContent).toContain(
      '01920000-0000-7000-8000-000000000001',
    );
  });

  it('claims every path the navigation offers', async () => {
    const harness = await RouterTestingHarness.create();

    for (const link of NAV_LINKS) {
      await harness.navigateByUrl(link.path);
      expect(TestBed.inject(Router).url).toBe(link.path);
      expect(harness.routeDebugElement?.componentInstance).not.toBeInstanceOf(NotFoundPage);
    }
  });

  it('answers an unclaimed URL with the not-found screen', async () => {
    const harness = await RouterTestingHarness.create();

    await harness.navigateByUrl('/nowhere/at/all', NotFoundPage);

    expect(harness.routeNativeElement?.textContent).toContain('Page introuvable');
  });

  it('titles the browser tab after the screen being shown', async () => {
    const harness = await RouterTestingHarness.create();

    await harness.navigateByUrl('/events', EventListPage);

    expect(document.title).toBe('Événements — alcoLoco');
  });
});
