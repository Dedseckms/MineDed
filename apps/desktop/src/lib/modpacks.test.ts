import { describe, expect, it } from "vitest";

import type { Instance } from "../bindings";
import { ApiError } from "../api/client";
import { PAGE_SIZE } from "./content";
import {
  DEFAULT_MODPACK_FILTERS,
  buildModpackQuery,
  findNewInstance,
  manualLinks,
  modpackVersionFilter,
} from "./modpacks";

const inst = (id: string, createdAt: string): Instance => ({
  id,
  name: id,
  mc: "1.21.1",
  loader: { kind: "fabric", version: "0.16.14" },
  java: { path: null, major: 21 },
  memory: { minMb: 1024, maxMb: 4096, auto: true },
  jvmArgs: [],
  preset: null,
  icon: null,
  createdAt,
  lastPlayed: null,
  playtimeSec: 0,
});

describe("modpack helpers", () => {
  it("builds a modpack search query", () => {
    const q = buildModpackQuery({ ...DEFAULT_MODPACK_FILTERS, text: "  optimized ", mc: "1.21.1", loader: "fabric" }, 24);
    expect(q).toEqual({
      source: "modrinth",
      kind: "modpack",
      text: "optimized",
      mc: "1.21.1",
      loader: "fabric",
      categories: [],
      sort: "downloads",
      offset: 24,
      limit: PAGE_SIZE,
    });
    expect(buildModpackQuery(DEFAULT_MODPACK_FILTERS, 0)).toMatchObject({ mc: null, loader: null });
  });

  it("filters versions by the catalog filters unless all are requested", () => {
    expect(modpackVersionFilter({ mc: "1.20.1", loader: "forge" }, false)).toEqual({ mc: "1.20.1", loader: "forge" });
    expect(modpackVersionFilter({ mc: "1.20.1", loader: "forge" }, true)).toEqual({ mc: null, loader: null });
  });

  it("extracts manual download links only from manualDownloadRequired", () => {
    const err = {
      code: "manualDownloadRequired" as const,
      message: "x",
      details: {
        url: "https://www.curseforge.com/a/files/1",
        url2: "https://www.curseforge.com/b/files/2, https://www.curseforge.com/a/files/1",
        fileName: "pack.zip",
      },
    };
    expect(manualLinks(new ApiError(err)).map((l) => l.url)).toEqual([
      "https://www.curseforge.com/a/files/1",
      "https://www.curseforge.com/b/files/2",
    ]);
    expect(manualLinks({ ...err, code: "network" })).toEqual([]);
    expect(manualLinks({ ...err, details: null })).toEqual([]);
    expect(manualLinks(null)).toEqual([]);
  });

  it("finds the instance created by an install", () => {
    const before = ["a", "b"];
    expect(findNewInstance(before, [inst("a", "2026-01-01"), inst("b", "2026-01-02")])).toBeNull();
    const after = [inst("a", "2026-01-01"), inst("c", "2026-02-01"), inst("d", "2026-03-01"), inst("b", "2026-01-02")];
    expect(findNewInstance(before, after)?.id).toBe("d");
  });
});
