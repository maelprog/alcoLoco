import { Injectable, computed, signal } from '@angular/core';

/**
 * The profile the user picked at launch.
 *
 * V1 has no authentication (SPEC.md §5.1): the current profile is a choice, not
 * an identity, so it lives in the browser and survives a reload — losing it on
 * every refresh would send the user back to the selection screen for nothing.
 * It carries no physiological parameter: those are read from the settings
 * version in force (SPEC.md §10.0-L), never cached here.
 */

/** Key the selection is stored under. Namespaced to survive other apps. */
export const CURRENT_PROFILE_STORAGE_KEY = 'alcoloco.current_profile';

/** Just enough of a profile to name the one in use and address the API. */
export interface CurrentProfile {
  /** Identifier of the profile, a UUID v7 minted by the API. */
  id: string;
  /** Name shown in the navigation. Spelled as the `profile` table spells it. */
  display_name: string;
}

@Injectable({ providedIn: 'root' })
export class CurrentProfileStore {
  private readonly selected = signal<CurrentProfile | null>(restore());

  /** The profile in use, or `null` when none has been picked yet. */
  readonly profile = this.selected.asReadonly();

  /** Whether a profile has been picked. */
  readonly hasProfile = computed(() => this.selected() !== null);

  /**
   * Records the profile the user picked.
   *
   * Only the identity is kept, whatever the caller hands over: anything else a
   * profile payload carries — starting with its physiological parameters —
   * would become a second source of truth, free to diverge from the settings
   * version in force from the first change on (SPEC.md §10.0-L).
   */
  select(profile: CurrentProfile): void {
    const selection: CurrentProfile = { id: profile.id, display_name: profile.display_name };
    this.selected.set(selection);
    persist(selection);
  }

  /** Forgets the current selection, sending the user back to the choice. */
  clear(): void {
    this.selected.set(null);
    persist(null);
  }
}

/**
 * Reads back a stored selection.
 *
 * Anything unreadable is treated as no selection: the storage is user-writable
 * and a decade of stale keys will outlive any given payload shape, so a bad
 * value must cost a re-selection, never a crash at boot.
 */
function restore(): CurrentProfile | null {
  const stored = read(CURRENT_PROFILE_STORAGE_KEY);
  if (stored === null) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(stored);
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      typeof (parsed as CurrentProfile).id === 'string' &&
      typeof (parsed as CurrentProfile).display_name === 'string'
    ) {
      const { id, display_name } = parsed as CurrentProfile;
      return { id, display_name };
    }
  } catch {
    // Not JSON at all. Same outcome as a payload of the wrong shape.
  }
  return null;
}

function persist(profile: CurrentProfile | null): void {
  try {
    if (profile === null) {
      localStorage.removeItem(CURRENT_PROFILE_STORAGE_KEY);
    } else {
      localStorage.setItem(CURRENT_PROFILE_STORAGE_KEY, JSON.stringify(profile));
    }
  } catch {
    // Storage disabled or full: the selection stays in memory for this tab.
  }
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
