/** Remembered guest chat identity (`social_guest_login`), so a restart resumes it. */
const KEY = "mineded.guestSession";

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Nick of the saved guest session, or null. */
export function savedGuestNick(): string | null {
  try {
    return storage()?.getItem(KEY) || null;
  } catch {
    return null;
  }
}

export function rememberGuest(nick: string): void {
  try {
    storage()?.setItem(KEY, nick);
  } catch {
    // Storage may be unavailable; the session still lives for this run.
  }
}

export function forgetGuest(): void {
  try {
    storage()?.removeItem(KEY);
  } catch {
    // Nothing to clean up.
  }
}
