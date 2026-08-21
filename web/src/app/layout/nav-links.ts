/**
 * The entries of the main navigation.
 *
 * They live outside the template so that the routing test can walk them and
 * check that each one actually resolves to a screen: a link pointing at a path
 * no route claims would otherwise render fine and quietly land the user on the
 * not-found page.
 */
export interface NavLink {
  /** Absolute path the entry navigates to. */
  path: string;
  /** What the user reads. */
  label: string;
}

export const NAV_LINKS: readonly NavLink[] = [
  { path: '/profiles', label: 'Profils' },
  { path: '/profile', label: 'Mon profil' },
  { path: '/events', label: 'Événements' },
];
