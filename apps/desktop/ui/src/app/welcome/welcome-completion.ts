/**
 * Whether this browser profile has continued past the welcome page. Its own
 * versioned key beside `xt.theme`: a later welcome that must be seen again
 * takes a new version rather than reinterpreting this one.
 */
export const welcomeStorageKey = 'xt.welcome.v1';
const completed = 'completed';

/** Unreadable storage reads as not completed; the startup fact still skips upgrades. */
export function welcomeCompleted(): boolean {
  try {
    return localStorage.getItem(welcomeStorageKey) === completed;
  } catch {
    return false;
  }
}

/** Returns whether the completion was stored; a failed write never blocks leaving. */
export function completeWelcome(): boolean {
  try {
    localStorage.setItem(welcomeStorageKey, completed);
    return true;
  } catch {
    return false;
  }
}
