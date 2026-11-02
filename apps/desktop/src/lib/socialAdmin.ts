// Social admin/guest wrappers around commands that may change shape between
// the hub and the desktop bindings. Keep the binding gap in one place.
import type { AppError } from "../bindings";
import { api } from "../api/client";

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

/**
 * Sends an account request with the login and password the guest picked.
 *
 * TEMPORARY: `bindings.ts` still has `social_request_account(note)` and
 * `social_issue_invite(request_id)` only. The server side of 1.5.1 expects
 * `social_request_account(login, password, note)` and
 * `social_create_account(request_id)`; until the bindings are regenerated the
 * extra arguments are passed through, so the dialog and the admin flow already
 * speak the final contract.
 */
export function socialRequestAccountWithCredentials(
  login: string,
  password: string,
  note: string,
): Promise<Res<null>> {
  const call = api.socialRequestAccount as unknown as (
    login: string,
    password: string,
    note: string,
  ) => Promise<Res<null>>;
  return call(login, password, note);
}

/** Creates the server account from an admin-approved request (replaces the invite). */
export function socialAdminCreateAccount(requestId: string): Promise<Res<null>> {
  // The current binding resolves to the old invite link (`string`); ignore it.
  const call = api.socialIssueInvite as unknown as (requestId: string) => Promise<Res<unknown>>;
  return call(requestId).then((res) =>
    res.status === "error" ? res : { status: "ok", data: null },
  );
}

/**
 * Whether the server account for this guest's request already exists. The hub
 * only reports it as the `accountCreated` flag of the guest's own profile; the
 * current bindings do not carry it, so it is read defensively until they do.
 */
export function profileAccountCreated(user: unknown): boolean {
  return (
    !!user &&
    typeof user === "object" &&
    (user as { accountCreated?: unknown }).accountCreated === true
  );
}
