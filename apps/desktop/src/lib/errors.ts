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
