// Pure helpers for the modpack catalog (ready-made instances from Modrinth / CurseForge).
import type { ContentFilter, Instance, InstanceId, LoaderKind, SearchQuery, SortBy } from "../bindings";
import { PAGE_SIZE, type SearchSource } from "./content";
import { toAppError } from "./errors";

/** Loaders a modpack can target (vanilla packs are not a thing). */
export const MODPACK_LOADERS = ["fabric", "quilt", "forge", "neoForge"] as const satisfies readonly LoaderKind[];
export type ModpackLoader = (typeof MODPACK_LOADERS)[number];

export interface ModpackFilters {
  source: SearchSource;
  text: string;
  /** `null` = any Minecraft version. */
  mc: string | null;
  /** `null` = any loader. */
  loader: ModpackLoader | null;
  sort: SortBy;
}

export const DEFAULT_MODPACK_FILTERS: ModpackFilters = {
  source: "modrinth",
  text: "",
  mc: null,
  loader: null,
  sort: "downloads",
};

/** Builds the `content_search` query (`kind = modpack`) for one page. */
export function buildModpackQuery(f: ModpackFilters, offset: number): SearchQuery {
  return {
    source: f.source,
    kind: "modpack",
    text: f.text.trim(),
    mc: f.mc,
    loader: f.loader,
    categories: [],
    sort: f.sort,
    offset,
    limit: PAGE_SIZE,
  };
}

/** Version filter of the project dialog: the catalog filters, or everything. */
export function modpackVersionFilter(f: Pick<ModpackFilters, "mc" | "loader">, all: boolean): ContentFilter {
  return all ? { mc: null, loader: null } : { mc: f.mc, loader: f.loader };
}

export interface ManualLink {
  /** Detail key the URL came from (stable React key). */
  key: string;
  url: string;
}

const URL_RE = /https?:\/\/[^\s,;]+/g;

/**
 * Download pages from a `manualDownloadRequired` error. The backend puts the
 * URL(s) into `details` (key names are not fixed), so every http(s) URL found in
 * any detail value is returned, deduplicated, in detail-key order.
 */
export function manualLinks(e: unknown): ManualLink[] {
  if (e == null) return [];
  const err = toAppError(e);
  if (err.code !== "manualDownloadRequired" || !err.details) return [];
  const seen = new Set<string>();
  const out: ManualLink[] = [];
  for (const [key, value] of Object.entries(err.details).sort(([a], [b]) => a.localeCompare(b))) {
    for (const url of value.match(URL_RE) ?? []) {
      if (seen.has(url)) continue;
      seen.add(url);
      out.push({ key: `${key}:${url}`, url });
    }
  }
  return out;
}

/**
 * The instance a modpack install created. The task carries no result, so the
 * list after the install is compared with a snapshot taken before it; the
 * newest unknown instance wins.
 */
export function findNewInstance(before: readonly InstanceId[], after: readonly Instance[]): Instance | null {
  const known = new Set(before);
  const fresh = after.filter((i) => !known.has(i.id));
  if (fresh.length === 0) return null;
  return fresh.reduce((a, b) => (b.createdAt > a.createdAt ? b : a));
}
