// Social admin/guest wrappers around commands that may change shape between
// the hub and the desktop bindings. Keep the binding gap in one place.
import type { AppError, SocialUser } from "../bindings";
import { api } from "../api/client";

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

/**
 * Sends an account request with the login and password the guest picked.
 */
export function socialRequestAccountWithCredentials(
  login: string,
  password: string,
  note: string,
): Promise<Res<null>> {
  return api.socialRequestAccount(login, password, note);
}

/** Creates the server account from an admin-approved request (replaces the invite). */
export function socialAdminCreateAccount(requestId: string): Promise<Res<null>> {
  return api.socialCreateAccount(requestId);
}

/** Whether the server account for this guest's request already exists. */
export function profileAccountCreated(user: SocialUser | null | undefined): boolean {
  return user?.accountCreated === true;
}
