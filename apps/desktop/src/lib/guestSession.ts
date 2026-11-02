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

/** Login the guest asked for, so the account screen can prefill it. */
const LOGIN_KEY = "mineded.guestRequestedLogin";

export function rememberRequestedLogin(login: string): void {
  try {
    localStorage.setItem(LOGIN_KEY, login);
  } catch {
    // Storage is optional: the account screen just asks for the login.
  }
}

export function requestedLogin(): string | null {
  try {
    return localStorage.getItem(LOGIN_KEY);
  } catch {
    return null;
  }
}
