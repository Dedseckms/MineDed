import type { AppError, ContentKind, InstalledContent, InstanceId } from "../bindings";
import { api } from "./client";

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

export type ContentAddLocal = (
  instance: InstanceId,
  kind: ContentKind,
  paths: string[],
) => Promise<Res<InstalledContent[]>>;

/** Copies local files into the instance folder for `kind` (`content_add_local`). */
export function contentAddLocal(
  instance: InstanceId,
  kind: ContentKind,
  paths: string[],
): Promise<Res<InstalledContent[]>> {
  return api.contentAddLocal(instance, kind, paths);
}
