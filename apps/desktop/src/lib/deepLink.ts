// Deep links `mineded://` / `mineup://` (tauri-plugin-deep-link): pure parsing.

export const DEEP_LINK_SCHEMES = ["mineded", "mineup"] as const;

export type DeepLink =
  | { kind: "open" }
  /** Room code for the Multiplayer join form. */
  | { kind: "join"; code: string }
  /** Drasl registration invite code for the own-server account form. */
  | { kind: "invite"; code: string };

const CODE = /^[A-Za-z0-9_-]{1,128}$/;

/**
 * `mineded://open`, `mineded://join/ABCD-1234`, `mineup://invite/<code>`
 * (trailing slashes and query strings are ignored). Unknown links → `null`.
 */
export function parseDeepLink(raw: string): DeepLink | null {
  const m = /^([a-z][a-z0-9+.-]*):\/\/([^?#]*)/i.exec(raw.trim());
  if (!m) return null;
  const scheme = (m[1] ?? "").toLowerCase();
  if (!(DEEP_LINK_SCHEMES as readonly string[]).includes(scheme)) return null;
  const parts = (m[2] ?? "").split("/").filter(Boolean);
  const action = (parts[0] ?? "").toLowerCase();
  let arg: string;
  try {
    arg = decodeURIComponent(parts[1] ?? "");
  } catch {
    return null;
  }
  if (action === "open" || action === "") return { kind: "open" };
  if ((action === "join" || action === "invite") && CODE.test(arg)) return { kind: action, code: arg };
  return null;
}

/**
 * Registration invite field: a raw code or a full Drasl invite URL
 * (`https://host/web/registration?invite=<code>`, also `mineded://invite/<code>`).
 * Returns the trimmed input when nothing matches (validation is the server's).
 */
export function extractInviteCode(input: string): string {
  const s = input.trim();
  const link = parseDeepLink(s);
  if (link?.kind === "invite") return link.code;
  if (/^https?:\/\//i.test(s)) {
    try {
      const code = new URL(s).searchParams.get("invite");
      if (code) return code.trim();
    } catch {
      // Not a URL after all: keep the raw text.
    }
  }
  return s;
}
