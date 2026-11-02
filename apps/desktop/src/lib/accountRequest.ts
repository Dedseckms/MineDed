// Account requests: a guest proposes a login and password, an admin creates the
// server account. The password lives only in the form state and goes straight
// to the backend; it is never stored, cached or logged here.
import type { ValidationKey } from "./validation";

/** Longest note an admin gets with an account request. */
export const ACCOUNT_NOTE_MAX = 300;
/** Shortest password Drasl accepts for a new account. */
export const ACCOUNT_PASSWORD_MIN = 6;
export const ACCOUNT_PASSWORD_MAX = 64;

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

/** Login chosen by a guest: `[A-Za-z0-9_]{3,16}` (same as a Yggdrasil name). */
export function validateAccountLogin(login: string): ValidationKey | null {
  if (login.length === 0) return "validation.required";
  if (!/^[A-Za-z0-9_]*$/.test(login)) return "validation.nick.chars";
  if (!/^[A-Za-z0-9_]{3,16}$/.test(login)) return "validation.nick.length";
  return null;
}

/** Password chosen by a guest; not trimmed on purpose. */
export function validateAccountPassword(password: string): ValidationKey | null {
  if (password.length === 0) return "validation.required";
  if (password.length < ACCOUNT_PASSWORD_MIN || password.length > ACCOUNT_PASSWORD_MAX)
    return "validation.password.range";
  return null;
}

/** `errors.reason.<reason>` for a refused account creation, if the code is known. */
const CREATE_FAILURES = ["usernameTaken", "weakPassword"] as const;
export type CreateAccountFailure = (typeof CREATE_FAILURES)[number];

/** Maps `details.reason` of a failed admin "create account" action. */
export function createAccountFailure(
  details: Record<string, string> | null | undefined,
): CreateAccountFailure | null {
  const reason = details?.reason;
  return CREATE_FAILURES.find((r) => r === reason) ?? null;
}
