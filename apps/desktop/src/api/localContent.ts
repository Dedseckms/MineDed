// Typed wrapper for `content_add_local` until `bindings.ts` is regenerated with it.
// The single cast below is the only place that knows the command may be missing.
import type { AppError, ContentKind, InstalledContent, InstanceId } from "../bindings";
import { api } from "./client";

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

export type ContentAddLocal = (
  instance: InstanceId,
  kind: ContentKind,
  paths: string[],
) => Promise<Res<InstalledContent[]>>;

/** Copies local files into the instance folder for `kind` (`content_add_local`). */
export async function contentAddLocal(
  instance: InstanceId,
  kind: ContentKind,
  paths: string[],
): Promise<Res<InstalledContent[]>> {
  const fn = (api as unknown as { contentAddLocal?: ContentAddLocal }).contentAddLocal;
  if (typeof fn !== "function") {
    return { status: "error", error: { code: "notImplemented", message: "content_add_local is not available", details: null } };
  }
  return fn(instance, kind, paths);
}
