// Pure helpers for the content (mods / resource packs / shaders) screens.
import type {
  ContentKind,
  ContentSourceKind,
  InstalledContent,
  Instance,
  LoaderKind,
  LoaderVersion,
  PackFormat,
  ResolvePlan,
  SearchPage,
  SearchQuery,
  SortBy,
} from "../bindings";

export const PAGE_SIZE = 12;

/** Catalog sources the user can search: Modrinth only (CurseForge is turned off). */
export const SEARCH_SOURCES = ["modrinth"] as const satisfies readonly ContentSourceKind[];
export type SearchSource = (typeof SEARCH_SOURCES)[number];

export const SEARCH_KINDS = ["mod", "resourcePack", "shader", "dataPack"] as const satisfies readonly ContentKind[];
export type SearchKind = (typeof SEARCH_KINDS)[number];

export const SORTS = ["relevance", "downloads", "updated", "newest"] as const satisfies readonly SortBy[];

export interface SearchFilters {
  source: SearchSource;
  kind: SearchKind;
  text: string;
  sort: SortBy;
  /** Restrict to the instance's Minecraft version and loader (default). */
  compatibleOnly: boolean;
}

/** Builds the `content_search` query for one page. */
export function buildSearchQuery(f: SearchFilters, instance: Pick<Instance, "mc" | "loader">, offset: number): SearchQuery {
  const loader = f.kind === "mod" && instance.loader.kind !== "vanilla" ? instance.loader.kind : null;
  return {
    source: f.source,
    kind: f.kind,
    text: f.text.trim(),
    mc: f.compatibleOnly ? instance.mc : null,
    loader: f.compatibleOnly ? loader : null,
    categories: [],
    sort: f.sort,
    offset,
    limit: PAGE_SIZE,
  };
}

/** Offset of the next page, or `undefined` when everything is loaded. */
export function nextOffset(page: SearchPage): number | undefined {
  const next = page.offset + page.items.length;
  return page.items.length > 0 && next < page.total ? next : undefined;
}

/** Recommended → stable → newest. */
export function pickLoaderVersion(list: LoaderVersion[]): string | null {
  return (list.find((v) => v.recommended) ?? list.find((v) => v.stable) ?? list[0])?.version ?? null;
}

export function installedOfKind(list: InstalledContent[], kind: ContentKind): InstalledContent[] {
  return list
    .filter((c) => c.kind === kind)
    .sort((a, b) => (a.title ?? a.fileName).localeCompare(b.title ?? b.fileName));
}

export function displayName(c: InstalledContent): string {
  return c.title ?? c.fileName;
}

export function filterInstalled(list: InstalledContent[], text: string): InstalledContent[] {
  const q = text.trim().toLowerCase();
  if (q === "") return list;
  return list.filter((c) => displayName(c).toLowerCase().includes(q) || c.fileName.toLowerCase().includes(q));
}

// ---- shader loader -----------------------------------------------------------

const SHADER_LOADER = /\b(iris|oculus)\b/i;

export type ShaderLoaderName = "iris" | "oculus";

/** Iris for Fabric/Quilt/NeoForge, Oculus for Forge, none for vanilla. */
export function shaderLoaderFor(loader: LoaderKind): ShaderLoaderName | null {
  if (loader === "forge") return "oculus";
  if (loader === "vanilla") return null;
  return "iris";
}

export function hasShaderLoader(installed: InstalledContent[], plan?: ResolvePlan): boolean {
  const inInstalled = installed.some(
    (c) => c.kind === "mod" && c.enabled && SHADER_LOADER.test(`${c.title ?? ""} ${c.fileName}`),
  );
  const inPlan = plan?.install.some((v) => SHADER_LOADER.test(`${v.name} ${v.fileName}`)) ?? false;
  return inInstalled || inPlan;
}

export type ShaderWarning = "none" | "missingLoader" | "vanilla";

/** Warning shown before installing shaders without Iris/Oculus. */
export function shaderWarning(plan: ResolvePlan, installed: InstalledContent[], loader: LoaderKind, shaderKinds: boolean): ShaderWarning {
  if (!shaderKinds) return "none";
  if (loader === "vanilla") return "vanilla";
  return hasShaderLoader(installed, plan) ? "none" : "missingLoader";
}

// ---- resolve plan -----------------------------------------------------------

export interface PlanSummary {
  totalBytes: number;
  installCount: number;
  canInstall: boolean;
}

export function summarizePlan(plan: ResolvePlan): PlanSummary {
  const totalBytes = plan.install.reduce((s, v) => s + v.size, 0);
  return {
    totalBytes,
    installCount: plan.install.length,
    canInstall: plan.conflicts.length === 0 && plan.install.length > 0,
  };
}

// ---- export -----------------------------------------------------------------

/** Export formats offered in the UI: Modrinth `.mrpack` only. */
export const PACK_FORMATS = ["mrpack"] as const satisfies readonly PackFormat[];

export function packExt(format: PackFormat): ".mrpack" | ".zip" {
  return format === "mrpack" ? ".mrpack" : ".zip";
}

/** Replaces a known pack extension with the one of `format` (or appends it). */
export function withPackExt(path: string, format: PackFormat): string {
  const p = path.trim();
  if (p === "") return p;
  const base = p.replace(/\.(mrpack|zip)$/i, "");
  return `${base}${packExt(format)}`;
}

// ---- formatting -------------------------------------------------------------

export function formatCount(n: number, locale: string): string {
  return new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 }).format(n);
}

const SIZE_UNITS = ["units.b", "units.kb", "units.mb", "units.gb"] as const;

/** Size as an i18n key (`units.*`, takes `{{value}}`) plus the formatted number. */
export function formatBytes(bytes: number, locale: string): { key: (typeof SIZE_UNITS)[number]; value: string } {
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < SIZE_UNITS.length - 1) {
    v /= 1024;
    i += 1;
  }
  return {
    key: SIZE_UNITS[i] ?? "units.b",
    value: new Intl.NumberFormat(locale, { maximumFractionDigits: i === 0 ? 0 : 1 }).format(v),
  };
}

// ---- virtualization ---------------------------------------------------------

/** Index range `[start, end)` of fixed-height rows visible in a viewport. */
export function visibleRange(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  overscan = 4,
): [number, number] {
  if (count === 0 || rowHeight <= 0) return [0, 0];
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(count, Math.ceil((scrollTop + viewport) / rowHeight) + overscan);
  return [start, Math.max(start, end)];
}

/** Kinds that can be added from disk, with the file extensions the picker offers. */
export const LOCAL_EXTS = {
  mod: ["jar"],
  resourcePack: ["zip"],
  shader: ["zip"],
  dataPack: ["zip"],
} as const satisfies Partial<Record<ContentKind, readonly string[]>>;

export type LocalKind = keyof typeof LOCAL_EXTS;

export function isLocalKind(kind: ContentKind): kind is LocalKind {
  return kind in LOCAL_EXTS;
}
