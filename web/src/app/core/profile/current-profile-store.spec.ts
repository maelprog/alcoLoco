import { TestBed } from '@angular/core/testing';

import { CURRENT_PROFILE_STORAGE_KEY, CurrentProfileStore } from './current-profile-store';

const ADA = { id: '01920000-0000-7000-8000-000000000001', display_name: 'Ada' };

function store(): CurrentProfileStore {
  TestBed.resetTestingModule();
  TestBed.configureTestingModule({});
  return TestBed.inject(CurrentProfileStore);
}

describe('CurrentProfileStore', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('starts without a profile', () => {
    const current = store();

    expect(current.profile()).toBeNull();
    expect(current.hasProfile()).toBe(false);
  });

  it('holds the profile the user picked', () => {
    const current = store();

    current.select(ADA);

    expect(current.profile()).toEqual(ADA);
    expect(current.hasProfile()).toBe(true);
  });

  it('survives a reload', () => {
    store().select(ADA);

    // A brand new store, as a page refresh would build it.
    expect(store().profile()).toEqual(ADA);
  });

  it('forgets the profile on clear, storage included', () => {
    const current = store();
    current.select(ADA);

    current.clear();

    expect(current.profile()).toBeNull();
    expect(localStorage.getItem(CURRENT_PROFILE_STORAGE_KEY)).toBeNull();
    expect(store().profile()).toBeNull();
  });

  it('ignores a stored value it cannot use', () => {
    // The storage is user-writable and outlives any payload shape: a bad value
    // must cost a re-selection, never a crash at boot.
    localStorage.setItem(CURRENT_PROFILE_STORAGE_KEY, 'not json at all');
    expect(store().profile()).toBeNull();

    localStorage.setItem(CURRENT_PROFILE_STORAGE_KEY, JSON.stringify({ id: 42 }));
    expect(store().profile()).toBeNull();
  });

  it('keeps nothing but the identity of the profile', () => {
    // Weight, height, sex and birth date are read from the settings version in
    // force at ingestion time (SPEC.md §10.0-L). A copy cached here would be a
    // second source of truth, free to diverge on the first change.
    store().select({ ...ADA, weight_kg: 70 } as never);

    const stored: unknown = JSON.parse(localStorage.getItem(CURRENT_PROFILE_STORAGE_KEY) ?? 'null');
    expect(Object.keys(stored as object).sort()).toEqual(['display_name', 'id']);
    expect(store().profile()).toEqual(ADA);
  });
});
