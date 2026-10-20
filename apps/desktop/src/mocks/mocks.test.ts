import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TaskEvent } from "../bindings";
import { ApiError, unwrap } from "../api/client";
import { createMockBus } from "./bus";
import { createMockCommands } from "./index";

const channel = (events: TaskEvent[]) =>
  ({ onmessage: (e: TaskEvent) => events.push(e) }) as unknown as Parameters<
    ReturnType<typeof createMockCommands>["instanceCreate"]
  >[1];

describe("mock commands", () => {
  it("returns realistic data", async () => {
    const api = createMockCommands();
    expect(await api.appVersion()).toBe("0.0.0-mock");
    expect(unwrap(await api.instanceList()).length).toBeGreaterThan(0);
    expect(unwrap(await api.accountList()).some((a) => a.isDefault)).toBe(true);
    const releases = unwrap(await api.versionsList({ kinds: ["release"], search: null }));
    expect(releases.every((v) => v.kind === "release")).toBe(true);
  });

  it("falls back to notImplemented for every other command", async () => {
    const api = createMockCommands();
    const res = await api.vpnState();
    expect(res.status).toBe("error");
    expect(() => unwrap(res)).toThrow(ApiError);
  });

  it("is not a thenable (can be resolved from a Promise)", async () => {
    const api = await Promise.resolve(createMockCommands());
    expect(await api.appVersion()).toBe("0.0.0-mock");
  });

  it("accepts per-test overrides", async () => {
    const api = createMockCommands({ settingsSet: async () => ({ status: "ok", data: null }) });
    expect((await api.settingsSet({} as never)).status).toBe("ok");
  });

  it("validates offline nicknames and emits accountChanged", async () => {
    const bus = createMockBus();
    const seen: number[] = [];
    bus.on("accountChanged", (list) => seen.push(list.length));
    const api = createMockCommands({}, { bus });
    expect((await api.accountOfflineAdd("no")).status).toBe("error");
    expect((await api.accountOfflineAdd("bad name")).status).toBe("error");
    expect(unwrap(await api.accountOfflineAdd("New_Player")).name).toBe("New_Player");
    expect((await api.accountOfflineAdd("new_player")).status).toBe("error");
    expect(seen).toEqual([3]);
  });

  it("yggdrasil login is not implemented yet", async () => {
    const api = createMockCommands();
    const res = await api.accountYggdrasilLogin("https://x", "u", "p");
    expect(res.status === "error" && res.error.code).toBe("notImplemented");
  });
});

describe("mock tasks", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("instanceCreate streams progress and adds the instance", async () => {
    const bus = createMockBus();
    const api = createMockCommands({}, { bus, tickMs: 10 });
    const events: TaskEvent[] = [];
    const id = unwrap(
      await api.instanceCreate(
        { name: "Test", mc: "1.21.10", loader: { kind: "vanilla", version: null }, preset: null, icon: null },
        channel(events),
      ),
    );
    await vi.advanceTimersByTimeAsync(2000);
    expect(events[0]).toMatchObject({ type: "started", id });
    expect(events.some((e) => e.type === "progress" && e.speedBps !== null)).toBe(true);
    expect(events.at(-1)).toEqual({ type: "finished", id, error: null });
    expect(unwrap(await api.instanceList()).some((i) => i.name === "Test")).toBe(true);
    expect(unwrap(await api.tasksList()).find((t) => t.id === id)?.status).toBe("succeeded");
  });

  it("launch → running → kill, with logs; cancel works", async () => {
    const bus = createMockBus();
    const states: string[] = [];
    let logLines = 0;
    bus.on("instanceState", (p) => states.push(p.state));
    bus.on("instanceLog", (p) => (logLines += p.lines.length));
    const api = createMockCommands({}, { bus, tickMs: 10 });
    const opts = { account: null, joinServer: null, room: null, offlineFast: false };
    unwrap(await api.instanceLaunch("inst-survival", opts, channel([])));
    await vi.advanceTimersByTimeAsync(2000);
    expect(unwrap(await api.instanceState("inst-survival"))).toBe("running");
    expect(logLines).toBeGreaterThan(0);
    expect((await api.instanceDelete("inst-survival")).status).toBe("error");
    unwrap(await api.instanceKill("inst-survival"));
    expect(states).toEqual(["preparing", "running", "stopped"]);

    const events: TaskEvent[] = [];
    const id = unwrap(await api.instanceLaunch("inst-classic", opts, channel(events)));
    unwrap(await api.taskCancel(id));
    await vi.advanceTimersByTimeAsync(200);
    const last = events.at(-1);
    expect(last?.type === "finished" && last.error?.code).toBe("cancelled");
    expect(unwrap(await api.instanceState("inst-classic"))).toBe("stopped");
  });
});
