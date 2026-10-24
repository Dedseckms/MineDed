// Borderless-fullscreen helper: Minecraft's own fullscreen is exclusive on
// Windows, so always-on-top windows (the voice overlay) can't be drawn over it.
// A client mod that turns F11 into borderless fullscreen fixes that.
import type { ContentFilter, ContentVersion, ContentVersionRef, InstalledContent, Instance, LoaderKind } from "../bindings";

/**
 * Modrinth project ids per loader, best first. Verified against the live
 * Modrinth API (`api.modrinth.com/v2/project/<id>` and `/version`) on
 * 2026-10-24; the first project with a version for the instance wins.
 */
const BORDERLESS_FULLSCREEN = "UIexcMP1"; // borderless-fullscreen (GPL-3.0): Fabric 1.20.1–26.x
const BORDERLESS_MINING = "kYq5qkSL"; // borderless-mining (MIT): Fabric/Quilt 1.15.2–1.20.2, Forge 1.12–
const BORDERLESS_MINING_UPDATED = "KxYAzYLi"; // borderless-mining-updated (MIT): Fabric/NeoForge 1.21.11+
const BORDERLESS_WINDOW = "CnalMsa0"; // borderless-window (MIT): Forge/NeoForge 1.14.4–1.21.11

export const BORDERLESS_MODS: Record<Exclude<LoaderKind, "vanilla">, readonly string[]> = {
  fabric: [BORDERLESS_FULLSCREEN, BORDERLESS_MINING, BORDERLESS_MINING_UPDATED],
  quilt: [BORDERLESS_MINING, BORDERLESS_FULLSCREEN],
  forge: [BORDERLESS_WINDOW, BORDERLESS_MINING],
  neoForge: [BORDERLESS_WINDOW, BORDERLESS_MINING_UPDATED],
};

/** Project titles as published on Modrinth (source text, not UI strings). */
export const BORDERLESS_TITLES: Record<string, string> = {
  [BORDERLESS_FULLSCREEN]: "Borderless Fullscreen",
  [BORDERLESS_MINING]: "Borderless Mining",
  [BORDERLESS_MINING_UPDATED]: "Borderless Mining Updated",
  [BORDERLESS_WINDOW]: "Borderless Window",
};

export function borderlessCandidates(loader: LoaderKind): readonly string[] {
  return loader === "vanilla" ? [] : BORDERLESS_MODS[loader];
}

/** An already installed borderless mod (by Modrinth project id), if any. */
export function installedBorderless(instance: Pick<Instance, "loader">, installed: readonly InstalledContent[]): InstalledContent | null {
  const ids = new Set(borderlessCandidates(instance.loader.kind));
  return installed.find((c) => c.source === "modrinth" && c.projectId !== null && ids.has(c.projectId)) ?? null;
}

type ListVersions = (projectId: string, filter: ContentFilter) => Promise<ContentVersion[]>;

/**
 * First candidate with a version for the instance's Minecraft version and
 * loader (`content_versions` filters on the backend). `null` = none fits.
 * Lookup errors of one candidate are skipped; if all fail, the last error is thrown.
 */
export async function findBorderlessMod(
  instance: Pick<Instance, "mc" | "loader">,
  listVersions: ListVersions,
): Promise<ContentVersionRef | null> {
  const filter: ContentFilter = { mc: instance.mc, loader: instance.loader.kind };
  let lastError: unknown = null;
  let anyOk = false;
  for (const projectId of borderlessCandidates(instance.loader.kind)) {
    try {
      const versions = await listVersions(projectId, filter);
      anyOk = true;
      const v = versions[0];
      if (v) return { source: "modrinth", projectId, versionId: v.versionId };
    } catch (e) {
      lastError = e;
    }
  }
  if (!anyOk && lastError !== null) throw lastError;
  return null;
}
