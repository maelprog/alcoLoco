import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { App } from './app';
import { DEV_MODE } from './core/dev-mode';
import { CurrentProfileStore } from './core/profile/current-profile-store';
import { NAV_LINKS } from './layout/nav-links';

async function configure(devMode: boolean): Promise<void> {
  TestBed.resetTestingModule();
  TestBed.configureTestingModule({
    imports: [App],
    providers: [
      provideRouter([]),
      provideHttpClient(),
      provideHttpClientTesting(),
      { provide: DEV_MODE, useValue: devMode },
    ],
  });
  await TestBed.compileComponents();
}

describe('App', () => {
  beforeEach(async () => {
    localStorage.clear();
    await configure(false);
  });

  it('should create the app', () => {
    const fixture = TestBed.createComponent(App);
    const app = fixture.componentInstance;
    expect(app).toBeTruthy();
  });

  it('should render the application title', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('h1')?.textContent).toContain('alcoLoco');
  });

  it('offers the main screens in the navigation', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const compiled = fixture.nativeElement as HTMLElement;

    const shown = Array.from(compiled.querySelectorAll('nav a')).map((link) => ({
      path: link.getAttribute('href'),
      label: link.textContent?.trim(),
    }));
    expect(shown).toEqual([
      { path: '/profiles', label: 'Profils' },
      { path: '/profile', label: 'Mon profil' },
      { path: '/events', label: 'Événements' },
    ]);
    expect(shown.map((link) => link.path)).toEqual(NAV_LINKS.map((link) => link.path));
  });

  it('gives the routed screen a container to render into', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const compiled = fixture.nativeElement as HTMLElement;

    expect(compiled.querySelector('main router-outlet')).not.toBeNull();
  });

  it('names the current profile once one is picked', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.textContent).toContain('Aucun profil sélectionné');

    TestBed.inject(CurrentProfileStore).select({ id: 'p-1', display_name: 'Ada' });
    await fixture.whenStable();

    expect(compiled.textContent).toContain('Ada');
    expect(compiled.textContent).not.toContain('Aucun profil sélectionné');
  });

  it('hides the health banner outside development', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();

    expect((fixture.nativeElement as HTMLElement).querySelector('app-health-indicator')).toBeNull();
    // Nothing was asked of the API either: the banner is the only caller.
    TestBed.inject(HttpTestingController).verify();
  });

  it('shows the health banner in development', async () => {
    await configure(true);
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();

    // The banner is mounted by the first render, and calls the API from there.
    const http = TestBed.inject(HttpTestingController);
    http
      .expectOne('/api/health')
      .flush({ status: 'ok', database: 'up', checked_at: '2026-08-20T21:04:05Z' });
    await fixture.whenStable();

    const banner = (fixture.nativeElement as HTMLElement).querySelector('app-health-indicator');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toContain('en ligne');
    http.verify();
  });
});
