/** Longest note an admin gets with an account request. */
export const ACCOUNT_NOTE_MAX = 300;

const PREFIX = "mineded.accountRequest.";

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * Whether this guest (by profile UUID) already asked for a real account.
 * Kept in `localStorage`, so the button stays disabled after a restart and the
 * hub is not bothered with duplicates.
 */
export function hasRequestedAccount(uuid: string): boolean {
  if (!uuid) return false;
  try {
    return storage()?.getItem(`${PREFIX}${uuid}`) === "1";
  } catch {
    return false;
  }
}

/** Remembers a sent request for `uuid`; storage failures are not fatal. */
export function markAccountRequested(uuid: string): void {
  if (!uuid) return;
  try {
    storage()?.setItem(`${PREFIX}${uuid}`, "1");
  } catch {
    // Private mode or a full quota: the in-memory state still disables the button.
  }
}

/** Trims a note and cuts it to the backend limit (counted in characters, not bytes). */
export function normalizeNote(note: string): string {
  return Array.from(note.trim()).slice(0, ACCOUNT_NOTE_MAX).join("");
}
