// Pure form validators. Each returns an i18n key under `validation.*` or `null`.
import type { ProxyCfg, ProxyKind } from "../bindings";

export type ValidationKey = `validation.${string}`;

// ---- Offline nickname -------------------------------------------------------

export const NICK_MIN = 3;
export const NICK_MAX = 16;
const NICK_CHARS = /^[A-Za-z0-9_]*$/;

/** Minecraft offline nickname: 3–16 chars of `[A-Za-z0-9_]`. */
export function validateNick(name: string): ValidationKey | null {
  if (name.length === 0) return "validation.required";
  if (!NICK_CHARS.test(name)) return "validation.nick.chars";
  if (name.length < NICK_MIN || name.length > NICK_MAX) return "validation.nick.length";
  return null;
}

// ---- Instance name ----------------------------------------------------------

export const INSTANCE_NAME_MAX = 64;
// Characters that are invalid in Windows file names (instance dirs are named after it).
// eslint-disable-next-line no-control-regex
const BAD_NAME_CHARS = /[<>:"/\\|?*\u0000-\u001f]/;

export function validateInstanceName(name: string): ValidationKey | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return "validation.required";
  if (trimmed.length > INSTANCE_NAME_MAX) return "validation.name.length";
  if (BAD_NAME_CHARS.test(trimmed)) return "validation.name.chars";
  return null;
}

// ---- Paths ------------------------------------------------------------------

// eslint-disable-next-line no-control-regex
const CONTROL_CHARS = /[\u0000-\u001f]/;
const ABSOLUTE_PATH = /^(?:[A-Za-z]:[\\/]|\\\\|\/|~[\\/])/;

/** Optional absolute path (empty = use default). */
export function validateOptionalPath(path: string): ValidationKey | null {
  const p = path.trim();
  if (p.length === 0) return null;
  if (CONTROL_CHARS.test(p)) return "validation.path.chars";
  if (!ABSOLUTE_PATH.test(p)) return "validation.path.absolute";
  return null;
}

/** Empty string → `null`, otherwise the trimmed path. */
export function pathOrNull(path: string): string | null {
  const p = path.trim();
  return p.length === 0 ? null : p;
}

/**
 * Required absolute path to a file with one of `exts` (lower-case, with dot).
 * Used for modpack import/export and skin PNGs.
 */
export function validateFilePath(path: string, exts: readonly string[]): ValidationKey | null {
  const p = path.trim();
  if (p.length === 0) return "validation.required";
  if (CONTROL_CHARS.test(p)) return "validation.path.chars";
  if (!ABSOLUTE_PATH.test(p)) return "validation.path.absolute";
  const base = (p.split(/[\\/]/).pop() ?? "").toLowerCase();
  if (!exts.some((e) => base.length > e.length && base.endsWith(e))) return "validation.path.ext";
  return null;
}

/** Modpack import accepts Modrinth `.mrpack` only (CurseForge is turned off). */
export const IMPORT_EXTS = [".mrpack"] as const;
export const SKIN_EXTS = [".png"] as const;

// ---- URLs -------------------------------------------------------------------

/** Optional http(s) URL (empty = not configured). */
export function validateOptionalUrl(url: string): ValidationKey | null {
  const u = url.trim();
  if (u.length === 0) return null;
  if (!/^https?:\/\//i.test(u)) return "validation.url.scheme";
  try {
    const parsed = new URL(u);
    if (!parsed.hostname || /\s/.test(u)) return "validation.url.invalid";
  } catch {
    return "validation.url.invalid";
  }
  return null;
}

// ---- Skins ------------------------------------------------------------------

export const SKIN_NAME_MAX = 32;

export function validateSkinName(name: string): ValidationKey | null {
  const n = name.trim();
  if (n.length === 0) return "validation.required";
  if (n.length > SKIN_NAME_MAX) return "validation.skinName.length";
  return null;
}

/** Default library name from a PNG path: file name without extension. */
export function skinNameFromPath(path: string): string {
  const base = path.trim().split(/[\\/]/).pop() ?? "";
  return base.replace(/\.png$/i, "").slice(0, SKIN_NAME_MAX);
}

// ---- Account server registration -------------------------------------------

export const PASSWORD_MIN = 8;

export interface RegisterDraft {
  invite: string;
  username: string;
  password: string;
  confirm: string;
}

export type RegisterErrors = Partial<Record<keyof RegisterDraft, ValidationKey>>;

export function validateRegister(d: RegisterDraft): RegisterErrors {
  const errors: RegisterErrors = {};
  if (d.invite.trim().length === 0) errors.invite = "validation.required";
  else if (!/^[A-Za-z0-9_-]{4,128}$/.test(d.invite.trim())) errors.invite = "validation.invite";
  const nick = validateNick(d.username);
  if (nick) errors.username = nick;
  if (d.password.length === 0) errors.password = "validation.required";
  else if (d.password.length < PASSWORD_MIN) errors.password = "validation.password.short";
  if (d.confirm !== d.password) errors.confirm = "validation.password.mismatch";
  return errors;
}

// ---- Memory -----------------------------------------------------------------

export const MEM_MIN_MB = 512;
export const MEM_STEP_MB = 256;

export interface MemoryErrors {
  minMb?: ValidationKey;
  maxMb?: ValidationKey;
}

export function validateMemory(
  minMb: number,
  maxMb: number,
  totalRamMb: number | null,
): MemoryErrors {
  const errors: MemoryErrors = {};
  if (!Number.isInteger(minMb) || minMb < MEM_MIN_MB) errors.minMb = "validation.memory.minTooLow";
  if (!Number.isInteger(maxMb) || maxMb < minMb) errors.maxMb = "validation.memory.maxBelowMin";
  else if (totalRamMb !== null && maxMb > totalRamMb) errors.maxMb = "validation.memory.exceedsRam";
  return errors;
}

// ---- JVM arguments ----------------------------------------------------------

export type ParseArgsResult = { ok: true; args: string[] } | { ok: false; error: ValidationKey };

/** Splits a command line into arguments, honouring single and double quotes. */
export function parseArgs(input: string): ParseArgsResult {
  const args: string[] = [];
  let cur = "";
  let inArg = false;
  let quote: '"' | "'" | null = null;
  for (const ch of input) {
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      inArg = true;
    } else if (/\s/.test(ch)) {
      if (inArg) {
        args.push(cur);
        cur = "";
        inArg = false;
      }
    } else {
      cur += ch;
      inArg = true;
    }
  }
  if (quote) return { ok: false, error: "validation.args.quote" };
  if (inArg) args.push(cur);
  return { ok: true, args };
}

/** Inverse of `parseArgs` for display in a text field. */
export function formatArgs(args: string[]): string {
  return args.map((a) => (a === "" || /[\s"']/.test(a) ? `"${a.replace(/"/g, "'")}"` : a)).join(" ");
}

// ---- Proxy ------------------------------------------------------------------

export type ProxyMode = "off" | ProxyKind;

/** Editable form state of `Settings.proxy`. Port is a string while typing. */
export interface ProxyDraft {
  mode: ProxyMode;
  host: string;
  port: string;
  username: string;
  password: string;
}

export type ProxyErrors = Partial<Record<"host" | "port" | "username" | "password", ValidationKey>>;

const HOST_LABEL = "[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?";
const HOSTNAME = new RegExp(`^${HOST_LABEL}(?:\\.${HOST_LABEL})*$`);
const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;
const IPV6 = /^\[?[0-9A-Fa-f:.]+\]?$/;
/** RFC 1929 limits SOCKS5 credentials to 255 bytes each. */
const CRED_MAX = 255;

export function isValidHost(host: string): boolean {
  if (host.length === 0 || host.length > 253) return false;
  const v4 = IPV4.exec(host);
  if (v4) return v4.slice(1).every((o) => Number(o) <= 255);
  if (host.includes(":")) return IPV6.test(host) && host.split(":").length >= 3;
  // All-numeric dotted names that failed the IPv4 check are invalid too.
  if (/^[\d.]+$/.test(host)) return false;
  return HOSTNAME.test(host);
}

export function parsePort(port: string): number | null {
  if (!/^\d{1,5}$/.test(port.trim())) return null;
  const n = Number(port.trim());
  return n >= 1 && n <= 65535 ? n : null;
}

export function validateProxy(draft: ProxyDraft): ProxyErrors {
  if (draft.mode === "off") return {};
  const errors: ProxyErrors = {};
  const host = draft.host.trim();
  if (host.length === 0) errors.host = "validation.required";
  else if (/^[a-z][a-z0-9+.-]*:\/\//i.test(host)) errors.host = "validation.proxy.scheme";
  else if (!isValidHost(host)) errors.host = "validation.proxy.host";

  if (draft.port.trim().length === 0) errors.port = "validation.required";
  else if (parsePort(draft.port) === null) errors.port = "validation.proxy.port";

  if (draft.username.length > CRED_MAX) errors.username = "validation.proxy.credLength";
  if (draft.password.length > CRED_MAX) errors.password = "validation.proxy.credLength";
  if (draft.password.length > 0 && draft.username.length === 0)
    errors.username = "validation.proxy.usernameRequired";
  return errors;
}

export function proxyToDraft(cfg: ProxyCfg | null): ProxyDraft {
  if (!cfg) return { mode: "off", host: "", port: "", username: "", password: "" };
  return {
    mode: cfg.kind,
    host: cfg.host,
    port: String(cfg.port),
    username: cfg.username ?? "",
    password: cfg.password ?? "",
  };
}

/** Converts a *valid* draft to the wire type (`null` = no proxy). */
export function draftToProxy(draft: ProxyDraft): ProxyCfg | null {
  if (draft.mode === "off") return null;
  return {
    kind: draft.mode,
    host: draft.host.trim(),
    port: parsePort(draft.port) ?? 0,
    username: draft.username.length > 0 ? draft.username : null,
    password: draft.password.length > 0 ? draft.password : null,
  };
}

export function hasErrors(errors: object): boolean {
  return Object.values(errors).some((v) => v !== undefined && v !== null);
}
