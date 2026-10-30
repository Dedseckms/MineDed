import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SearchQuery, TaskEvent } from "../bindings";
import { unwrap } from "../api/client";
import { createMockCommands } from "./index";

const channel = (events: TaskEvent[] = []) =>
  ({ onmessage: (e: TaskEvent) => events.push(e) }) as unknown as Parameters<
    ReturnType<typeof createMockCommands>["modpackInstall"]
  >[3];

const query = (over: Partial<SearchQuery> = {}): SearchQuery => ({
  source: "modrinth",
  kind: "modpack",
  text: "",
  mc: null,
  loader: null,
  categories: [],
  sort: "downloads",
  offset: 0,
  limit: 20,
  ...over,
});

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("mock modpacks", () => {
  it("searches modpacks by source, version and loader", async () => {
    const api = createMockCommands();
    const all = unwrap(await api.contentSearch(query()));
    expect(all.items.length).toBeGreaterThanOrEqual(3);
    expect(all.items.every((p) => p.kind === "modpack" && p.source === "modrinth")).toBe(true);
    const neo = unwrap(await api.contentSearch(query({ loader: "neoForge" })));
    expect(neo.items.map((p) => p.slug)).toEqual(["better-mc-neoforge"]);
    const old = unwrap(await api.contentSearch(query({ mc: "1.12.2" })));
    expect(old.items).toEqual([]);
    const versions = unwrap(await api.contentVersions("modrinth", "1KVo5zza", { mc: "1.21.1", loader: "fabric" }));
    expect(versions.length).toBeGreaterThan(0);
    expect(versions.every((v) => v.mcVersions.includes("1.21.1") && v.fileName.endsWith(".mrpack"))).toBe(true);
  });

  it("installs a modpack as a new instance", async () => {
    const api = createMockCommands({}, { tickMs: 5 });
    const events: TaskEvent[] = [];
    unwrap(await api.modpackInstall("modrinth", "1KVo5zza", null, channel(events)));
    await vi.advanceTimersByTimeAsync(2000);
    const finished = events.find((e) => e.type === "finished");
    expect(finished?.type === "finished" && finished.error).toBeNull();
    expect(events[0]).toMatchObject({ type: "started", kind: "instanceImport", titleKey: "tasks.modpackInstall" });
    const created = unwrap(await api.instanceList()).find((i) => i.name === "Fabulously Optimized");
    expect(created?.loader.kind).toBe("fabric");
    expect(created?.loader.version).not.toBeNull();
    expect((await api.modpackInstall("modrinth", "nope", null, channel())).status).toBe("error");
    expect((await api.modpackInstall("modrinth", "1KVo5zza", "bad-version", channel())).status).toBe("error");
  });

  it("installs the selected CurseForge pack using numeric IDs", async () => {
    const api = createMockCommands({}, { tickMs: 5 });
    const versions = unwrap(await api.contentVersions("curseForge", "447315", { mc: null, loader: null }));
    expect(versions[0]?.source).toBe("curseForge");
    expect(versions[0]?.versionId).toMatch(/^\d+$/);
    const events: TaskEvent[] = [];
    unwrap(await api.modpackInstall("curseForge", "447315", versions[0]?.versionId ?? null, channel(events)));
    await vi.advanceTimersByTimeAsync(2000);
    expect(events.find((e) => e.type === "finished")).toMatchObject({ error: null });
    expect(unwrap(await api.instanceList()).some((i) => i.name === "Better MC (CurseForge)")).toBe(true);
  });
});
