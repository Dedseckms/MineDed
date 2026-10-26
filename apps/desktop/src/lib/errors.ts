import type { AppError, ErrorCode, TaskKind } from "../bindings";
import { ApiError } from "../api/client";

// Exhaustive lists (a missing variant fails typecheck via `satisfies`).
const ERROR_CODE_MAP = {
  network: true,
  blocked: true,
  hashMismatch: true,
  notFound: true,
  authExpired: true,
  authRequired: true,
  javaMissing: true,
  loaderInstall: true,
  manualDownloadRequired: true,
  cancelled: true,
  io: true,
  invalidInput: true,
  conflict: true,
  permissionDenied: true,
  rateLimited: true,
  notImplemented: true,
  internal: true,
} satisfies Record<ErrorCode, true>;

export const ERROR_CODES = Object.keys(ERROR_CODE_MAP) as ErrorCode[];

const TASK_KIND_MAP = {
  javaInstall: true,
  instanceCreate: true,
  instanceRepair: true,
  instanceImport: true,
  instanceExport: true,
  launch: true,
  contentInstall: true,
  contentUpdate: true,
  presetApply: true,
  accountLogin: true,
  vpnUpdate: true,
  appUpdate: true,
} satisfies Record<TaskKind, true>;

export const TASK_KINDS = Object.keys(TASK_KIND_MAP) as TaskKind[];

/** Extracts an `AppError` from anything thrown by `unwrap` / commands. */
export function toAppError(e: unknown): AppError {
  if (e instanceof ApiError) return e.error;
  if (e && typeof e === "object" && "code" in e && typeof e.code === "string" && e.code in ERROR_CODE_MAP) {
    const err = e as Partial<AppError>;
    return { code: err.code as ErrorCode, message: err.message ?? "", details: err.details ?? null };
  }
  return { code: "internal", message: e instanceof Error ? e.message : String(e), details: null };
}

export function errorKey(e: unknown): `errors.${ErrorCode}` {
  return `errors.${toAppError(e).code}`;
}

export function isNotImplemented(e: unknown): boolean {
  return e != null && toAppError(e).code === "notImplemented";
}

/** `details.reason` values with a friendly message (`errors.reason.<reason>`). */
export const KNOWN_REASONS = [
  "authModeMismatch",
  "yggdrasilAccountRequired",
  "instanceRequired",
  "relayUnreachable",
  "rateLimited",
  "curseforgeDisabled",
  "vpnUnavailable",
  "uacDeclined",
  "helperFailed",
  "vpnNotRunning",
  "hostTooOld",
] as const;
export type KnownReason = (typeof KNOWN_REASONS)[number];

/** Detail keys shown first in the "More" block, in this order. */
const DETAIL_ORDER = ["reason", "step", "source", "status", "hint"];

export interface ErrorView {
  code: ErrorCode;
  /** i18n key of the main message: a known reason wins over the code. */
  messageKey: string;
  params: Record<string, string>;
  /** All details, the well-known ones first. */
  details: Array<[string, string]>;
  /** Backend suggests a proxy / VPN (`details.hint = useProxy`). */
  suggestProxy: boolean;
  /** Plain text for the "Copy" button (code, developer message, details). */
  copyText: string;
}

/** Backend errors without a `reason` detail, recognised by their developer message. */
function reasonFromMessage(code: ErrorCode, message: string): KnownReason | undefined {
  if (code === "invalidInput" && /vpn is not running/i.test(message)) return "vpnNotRunning";
  return undefined;
}

function knownReason(details: Record<string, string>, code: ErrorCode, message: string): KnownReason | null {
  const r =
    details.reason ??
    details.hubCode ??
    (code === "rateLimited" ? "rateLimited" : undefined) ??
    reasonFromMessage(code, message);
  return r && (KNOWN_REASONS as readonly string[]).includes(r) ? (r as KnownReason) : null;
}

/** Everything the UI shows about an error: translated message, details and actions. */
export function errorView(e: unknown): ErrorView {
  const err = toAppError(e);
  const raw = err.details ?? {};
  const reason = knownReason(raw, err.code, err.message);
  const params: Record<string, string> = {};
  let messageKey: string = `errors.${err.code}`;
  if (reason === "rateLimited") {
    const retry = raw.retryAfter;
    messageKey = retry && /^\d+$/.test(retry) ? "errors.reason.rateLimitedRetry" : "errors.reason.rateLimited";
    if (retry) params.seconds = retry;
  } else if (reason) {
    messageKey = `errors.reason.${reason}`;
  }
  const rank = (k: string) => {
    const i = DETAIL_ORDER.indexOf(k);
    return i === -1 ? DETAIL_ORDER.length : i;
  };
  const details = Object.entries(raw).sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b));
  const copyText = [
    `code: ${err.code}`,
    ...(err.message ? [`message: ${err.message}`] : []),
    ...details.map(([k, v]) => `${k}: ${v}`),
  ].join("\n");
  return { code: err.code, messageKey, params, details, suggestProxy: raw.hint === "useProxy", copyText };
}
