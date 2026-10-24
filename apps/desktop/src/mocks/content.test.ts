import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SearchQuery, TaskEvent } from "../bindings";
import { unwrap } from "../api/client";
import { createMockBus } from "./bus";
import { createMockCommands } from "./index";
import { encodePng } from "./skins";

const channel = (events: TaskEvent[] = []) =>
  ({ onmessage: (e: TaskEvent) => events.push(e) }) as unknown as Parameters<
    ReturnType<typeof createMockCommands>["contentInstall"]
  >[2];

const query = (over: Partial<SearchQuery> = {}): SearchQuery => ({
  source: "modrinth",
  kind: "mod",
  text: "",
  mc: "1.21.1",
  loader: "fabric",
  categories: [],
  sort: "downloads",
  offset: 0,
  limit: 10,
  ...over,
});

describe("mock catalog", () => {
  it("pages, filters and sorts search results", async () => {
    const api = createMockCommands();
    const p1 = unwrap(await api.contentSearch(query()));
    expect(p1.items).toHaveLength(10);
    expect(p1.total).toBeGreaterThan(10);
    const p2 = unwrap(await api.contentSearch(query({ offset: 10 })));
    expect(p2.offset).toBe(10);
    expect(p1.items[0]?.downloads).toBeGreaterThanOrEqual(p1.items[1]?.downloads ?? 0);
    // Forge-only Oculus is filtered out for Fabric.
    expect([...p1.items, ...p2.items].some((p) => p.slug === "oculus")).toBe(false);
    const text = unwrap(await api.contentSearch(query({ text: "sodium", sort: "relevance" })));
    expect(text.items[0]?.title).toBe("Sodium");
    const shaders = unwrap(await api.contentSearch(query({ kind: "shader" })));
    expect(shaders.items.every((p) => p.kind === "shader" && p.source === "modrinth")).toBe(true);
    // CurseForge is turned off: requests fail with a known reason.
    const cf = await api.contentSearch(query({ source: "curseForge" }));
    expect(cf.status === "error" && cf.error.details?.reason).toBe("curseforgeDisabled");
    expect((await api.contentSearch(query({ limit: 0 }))).status).toBe("error");
  });

  it("lists project versions filtered by instance", async () => {
    const api = createMockCommands();
    const all = unwrap(await api.contentVersions("modrinth", "AANobbMI", { mc: null, loader: null }));
    const compat = unwrap(await api.contentVersions("modrinth", "AANobbMI", { mc: "1.21.1", loader: "fabric" }));
    expect(all.length).toBeGreaterThan(compat.length);
    expect(compat.every((v) => v.mcVersions.includes("1.21.1"))).toBe(true);
  });

  it("returns loader versions per Minecraft version", async () => {
    const api = createMockCommands();
    expect(unwrap(await api.loaderVersions("fabric", "1.21.1")).some((v) => v.recommended)).toBe(true);
    expect(unwrap(await api.loaderVersions("neoForge", "1.20.1"))).toEqual([]);
    expect(unwrap(await api.loaderVersions("forge", "25w41a"))).toEqual([]);
    expect(unwrap(await api.loaderVersions("vanilla", "1.21.1"))).toEqual([]);
  });
});

describe("mock resolve", () => {
  it("adds required dependencies and skips installed ones", async () => {
    const api = createMockCommands();
    // Sodium Extra needs Sodium (installed) and Fabric API (installed).
    const plan = unwrap(await api.contentResolve("inst-fabric", [{ source: "modrinth", projectId: "PtjYWJkn", versionId: null }]));
    expect(plan.install.map((v) => v.projectId)).toEqual(["PtjYWJkn"]);
    expect(plan.alreadyInstalled.sort()).toEqual(["AANobbMI", "P7dR8mSH"]);
    // The same version of an installed resource pack is a no-op.
    const empty = unwrap(await api.contentResolve("inst-survival", [{ source: "modrinth", projectId: "50dA9Sha", versionId: null }]));
    expect(empty.alreadyInstalled).toEqual(["50dA9Sha"]);
  });

  it("reports conflicts", async () => {
    const api = createMockCommands();
    const canvas = await api.contentResolve("inst-fabric", [{ source: "modrinth", projectId: "fRiHVvU7", versionId: "fRiHVvU7-1.20.1-0" }]);
    expect(unwrap(canvas).conflicts).toEqual([{ projectId: "fRiHVvU7", withProjectId: "AANobbMI", reason: "incompatible" }]);
    expect(unwrap(canvas).manual).toEqual([]);
    const none = await api.contentResolve("inst-survival", [{ source: "modrinth", projectId: "AANobbMI", versionId: null }]);
    expect(none.status === "error" && none.error.code).toBe("notFound");
  });
});

describe("mock content tasks", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("installs a plan, toggles and removes files", async () => {
    const api = createMockCommands({}, { tickMs: 5 });
    const plan = unwrap(await api.contentResolve("inst-fabric", [{ source: "modrinth", projectId: "PtjYWJkn", versionId: null }]));
    const events: TaskEvent[] = [];
    unwrap(await api.contentInstall("inst-fabric", plan, channel(events)));
    await vi.advanceTimersByTimeAsync(1000);
    expect(events.at(-1)).toMatchObject({ type: "finished", error: null });
    const list = unwrap(await api.contentList("inst-fabric"));
    const extra = list.find((c) => c.projectId === "PtjYWJkn");
    expect(extra?.enabled).toBe(true);
    if (!extra) throw new Error("not installed");
    unwrap(await api.contentSetEnabled("inst-fabric", extra.fileName, false));
    expect(unwrap(await api.contentList("inst-fabric")).find((c) => c.fileName === extra.fileName)?.enabled).toBe(false);
    unwrap(await api.contentRemove("inst-fabric", extra.fileName));
    expect(unwrap(await api.contentList("inst-fabric")).some((c) => c.fileName === extra.fileName)).toBe(false);
    expect((await api.contentRemove("inst-fabric", "nope.jar")).status).toBe("error");
  });

  it("refuses plans with conflicts", async () => {
    const api = createMockCommands();
    const res = await api.contentInstall(
      "inst-fabric",
      { install: [], alreadyInstalled: [], conflicts: [{ projectId: "a", withProjectId: "b", reason: "x" }], manual: [] },
      channel(),
    );
    expect(res.status === "error" && res.error.code).toBe("conflict");
  });

  it("updates all, applies presets, imports and exports", async () => {
    const api = createMockCommands({}, { tickMs: 5 });
    unwrap(await api.contentUpdateAll("inst-fabric", channel()));
    unwrap(await api.presetApply("inst-classic", "potato", channel()));
    unwrap(await api.instanceImport("C:\\Downloads\\Cool Pack.mrpack", channel()));
    unwrap(await api.instanceExport("inst-fabric", "mrpack", "C:\\out\\pack.mrpack", channel()));
    await vi.advanceTimersByTimeAsync(2000);
    expect(unwrap(await api.contentList("inst-fabric")).some((c) => c.updateAvailable)).toBe(false);
    const instances = unwrap(await api.instanceList());
    expect(instances.find((i) => i.id === "inst-classic")?.preset).toBe("potato");
    expect(instances.find((i) => i.name === "Cool Pack")?.loader.kind).toBe("fabric");
    expect((await api.instanceImport("pack.rar", channel())).status).toBe("error");
    expect((await api.instanceExport("inst-fabric", "launcherZip", "C:\\out\\x.mrpack", channel())).status).toBe("error");
  });

  it("creates instances with a loader version", async () => {
    const api = createMockCommands({}, { tickMs: 5 });
    const spec = { name: "Fab", mc: "1.21.1", preset: null, icon: null };
    expect((await api.instanceCreate({ ...spec, loader: { kind: "fabric", version: null } }, channel())).status).toBe("error");
    expect((await api.instanceCreate({ ...spec, loader: { kind: "fabric", version: "9.9.9" } }, channel())).status).toBe("error");
    unwrap(await api.instanceCreate({ ...spec, loader: { kind: "fabric", version: "0.16.14" } }, channel()));
    await vi.advanceTimersByTimeAsync(3000);
    expect(unwrap(await api.instanceList()).find((i) => i.name === "Fab")?.loader).toEqual({ kind: "fabric", version: "0.16.14" });
  });
});

describe("mock skins", () => {
  it("encodes a valid PNG", () => {
    const png = encodePng(2, 2, new Uint8Array(16).fill(255));
    expect([...png.slice(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  });

  it("adds, applies and removes skins; capes per account", async () => {
    const bus = createMockBus();
    const changes: number[] = [];
    bus.on("accountChanged", (l) => changes.push(l.length));
    const api = createMockCommands({}, { bus });
    const lib = unwrap(await api.skinLibrary());
    expect(lib.length).toBeGreaterThanOrEqual(4);
    expect(lib.every((s) => s.url.startsWith("data:image/png;base64,"))).toBe(true);

    expect((await api.skinAdd("skin.png", "X", "slim")).status).toBe("error");
    expect((await api.skinAdd("C:\\a\\skin.jpg", "X", "slim")).status).toBe("error");
    const added = unwrap(await api.skinAdd("C:\\a\\skin.png", "Mine", "slim"));
    expect(added).toMatchObject({ name: "Mine", model: "slim", localOnly: true });

    unwrap(await api.skinApply("acc-steve", added.id));
    expect(unwrap(await api.accountList()).find((a) => a.id === "acc-steve")?.skinUrl).toBe(added.url);
    expect(changes).toEqual([2]);

    const capes = unwrap(await api.capeList("acc-steve"));
    expect(capes.some((c) => c.active)).toBe(false);
    unwrap(await api.capeSet("acc-steve", capes[0]?.id ?? null));
    expect(unwrap(await api.capeList("acc-steve")).find((c) => c.active)?.id).toBe(capes[0]?.id);
    expect(unwrap(await api.capeList("acc-alex")).some((c) => c.active)).toBe(false);
    expect((await api.capeSet("acc-steve", "nope")).status).toBe("error");

    unwrap(await api.skinRemove(added.id));
    expect(unwrap(await api.skinLibrary()).some((s) => s.id === added.id)).toBe(false);
  });

  it("yggdrasil registration is not implemented yet", async () => {
    const api = createMockCommands();
    const res = await api.accountYggdrasilRegister("https://x", "INVITE", "u", "p");
    expect(res.status === "error" && res.error.code).toBe("notImplemented");
  });
});
