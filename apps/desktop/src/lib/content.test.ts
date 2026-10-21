import { describe, expect, it } from "vitest";

import type { ContentVersion, InstalledContent, ResolvePlan } from "../bindings";
import { frontViewBlits, headBlits } from "../features/skins/skinRender";
import {
  PAGE_SIZE,
  buildSearchQuery,
  filterInstalled,
  formatBytes,
  hasShaderLoader,
  installedOfKind,
  nextOffset,
  pickLoaderVersion,
  shaderLoaderFor,
  shaderWarning,
  summarizePlan,
  visibleRange,
  withPackExt,
} from "./content";

const filters = { source: "modrinth", kind: "mod", text: "  sodium ", sort: "relevance", compatibleOnly: true } as const;
const fabric = { mc: "1.21.1", loader: { kind: "fabric", version: "0.16.14" } } as const;
const vanilla = { mc: "1.21.10", loader: { kind: "vanilla", version: null } } as const;

const item = (fileName: string, over: Partial<InstalledContent> = {}): InstalledContent => ({
  fileName,
  kind: "mod",
  enabled: true,
  source: "modrinth",
  projectId: null,
  versionId: null,
  title: null,
  versionNumber: null,
  updateAvailable: false,
  ...over,
});

const version = (over: Partial<ContentVersion>): ContentVersion => ({
  source: "modrinth",
  projectId: "p",
  versionId: "v",
  name: "Thing 1.0",
  versionNumber: "1.0",
  mcVersions: ["1.21.1"],
  loaders: ["fabric"],
  published: "2026-01-01T00:00:00Z",
  fileName: "thing-1.0.jar",
  size: 1000,
  sha1: null,
  dependencies: [],
  manualDownload: false,
  ...over,
});

const plan = (over: Partial<ResolvePlan> = {}): ResolvePlan => ({
  install: [],
  alreadyInstalled: [],
  conflicts: [],
  manual: [],
  ...over,
});

describe("search query", () => {
  it("uses instance version and loader by default", () => {
    expect(buildSearchQuery(filters, fabric, 40)).toEqual({
      source: "modrinth",
      kind: "mod",
      text: "sodium",
      mc: "1.21.1",
      loader: "fabric",
      categories: [],
      sort: "relevance",
      offset: 40,
      limit: PAGE_SIZE,
    });
  });

  it("drops filters when compatibility is off; no loader for packs or vanilla", () => {
    const q = buildSearchQuery({ ...filters, compatibleOnly: false }, fabric, 0);
    expect([q.mc, q.loader]).toEqual([null, null]);
    expect(buildSearchQuery({ ...filters, kind: "shader" }, fabric, 0).loader).toBeNull();
    expect(buildSearchQuery(filters, vanilla, 0).loader).toBeNull();
  });

  it("computes the next page offset", () => {
    expect(nextOffset({ items: new Array(20).fill(null), total: 45, offset: 0 })).toBe(20);
    expect(nextOffset({ items: new Array(5).fill(null), total: 45, offset: 40 })).toBeUndefined();
    expect(nextOffset({ items: [], total: 45, offset: 60 })).toBeUndefined();
  });
});

describe("loader versions", () => {
  const v = (version: string, stable: boolean, recommended: boolean) => ({
    loader: "fabric" as const,
    version,
    mc: "1.21.1",
    stable,
    recommended,
  });
  it("prefers recommended, then stable, then newest", () => {
    expect(pickLoaderVersion([v("3", false, false), v("2", true, false), v("1", true, true)])).toBe("1");
    expect(pickLoaderVersion([v("3", false, false), v("2", true, false)])).toBe("2");
    expect(pickLoaderVersion([v("3", false, false)])).toBe("3");
    expect(pickLoaderVersion([])).toBeNull();
  });
});

describe("installed content", () => {
  const list = [
    item("b.jar", { title: "Beta" }),
    item("pack.zip", { kind: "resourcePack" }),
    item("a.jar", { title: "Alpha" }),
    item("zz.jar"),
  ];
  it("groups by kind and sorts by display name", () => {
    expect(installedOfKind(list, "mod").map((c) => c.fileName)).toEqual(["a.jar", "b.jar", "zz.jar"]);
    expect(installedOfKind(list, "shader")).toEqual([]);
  });
  it("filters by title or file name", () => {
    expect(filterInstalled(list, " alp ").map((c) => c.fileName)).toEqual(["a.jar"]);
    expect(filterInstalled(list, "ZZ").map((c) => c.fileName)).toEqual(["zz.jar"]);
    expect(filterInstalled(list, "")).toHaveLength(4);
  });
});

describe("shader loader", () => {
  it("picks Iris or Oculus by mod loader", () => {
    expect(shaderLoaderFor("fabric")).toBe("iris");
    expect(shaderLoaderFor("neoForge")).toBe("iris");
    expect(shaderLoaderFor("forge")).toBe("oculus");
    expect(shaderLoaderFor("vanilla")).toBeNull();
  });

  it("detects an enabled Iris/Oculus in the instance or in the plan", () => {
    expect(hasShaderLoader([item("iris-1.8.jar")])).toBe(true);
    expect(hasShaderLoader([item("iris-1.8.jar", { enabled: false })])).toBe(false);
    expect(hasShaderLoader([item("x.jar", { title: "Oculus" })])).toBe(true);
    expect(hasShaderLoader([item("polaris.jar")])).toBe(false);
    expect(hasShaderLoader([], plan({ install: [version({ name: "Iris Shaders 1.8" })] }))).toBe(true);
  });

  it("warns only for shader installs without a loader", () => {
    expect(shaderWarning(plan(), [], "fabric", false)).toBe("none");
    expect(shaderWarning(plan(), [], "vanilla", true)).toBe("vanilla");
    expect(shaderWarning(plan(), [], "fabric", true)).toBe("missingLoader");
    expect(shaderWarning(plan(), [item("iris.jar")], "fabric", true)).toBe("none");
  });
});

describe("plan summary", () => {
  it("sums sizes and blocks installs with conflicts", () => {
    const p = plan({ install: [version({ size: 100 }), version({ versionId: "w", size: 50 })] });
    expect(summarizePlan(p)).toEqual({ totalBytes: 150, installCount: 2, canInstall: true });
    const c = { ...p, conflicts: [{ projectId: "a", withProjectId: "b", reason: "x" }] };
    expect(summarizePlan(c).canInstall).toBe(false);
    expect(summarizePlan(plan()).canInstall).toBe(false);
  });
});

describe("formatting & helpers", () => {
  it("swaps pack extensions", () => {
    expect(withPackExt("C:\\a\\pack.mrpack", "launcherZip")).toBe("C:\\a\\pack.zip");
    expect(withPackExt("C:\\a\\pack.zip", "mrpack")).toBe("C:\\a\\pack.mrpack");
    expect(withPackExt("C:\\a\\pack", "curseForgeZip")).toBe("C:\\a\\pack.zip");
    expect(withPackExt("  ", "mrpack")).toBe("");
  });

  it("formats sizes as unit keys", () => {
    expect(formatBytes(512, "en")).toEqual({ key: "units.b", value: "512" });
    expect(formatBytes(1536, "en")).toEqual({ key: "units.kb", value: "1.5" });
    expect(formatBytes(5 * 1024 * 1024, "en").key).toBe("units.mb");
  });

  it("computes the visible window", () => {
    expect(visibleRange(0, 300, 60, 140, 2)).toEqual([0, 7]);
    expect(visibleRange(600, 300, 60, 140, 2)).toEqual([8, 17]);
    expect(visibleRange(8000, 300, 60, 140, 2)[1]).toBe(140);
    expect(visibleRange(0, 300, 60, 0)).toEqual([0, 0]);
  });
});

describe("skin 2D render layout", () => {
  it("uses 3px arms for slim and mirrors limbs on legacy skins", () => {
    const slim = frontViewBlits("slim", false);
    expect(slim.find((b) => b.sx === 44 && b.sy === 20)?.w).toBe(3);
    const legacy = frontViewBlits("classic", true);
    expect(legacy.some((b) => b.sy >= 36)).toBe(false);
    expect(legacy.filter((b) => b.flip)).toHaveLength(2);
    // Every blit stays inside the 16×32 canvas.
    for (const b of [...slim, ...legacy]) {
      expect(b.dx >= 0 && b.dx + b.w <= 16 && b.dy >= 0 && b.dy + b.h <= 32).toBe(true);
    }
    expect(headBlits()).toHaveLength(2);
  });
});
