import { beforeEach, describe, expect, it } from "vitest";

import type { Instance, InstanceSig } from "../bindings";
import { useHostPackStore } from "../stores/hostPackStore";
import { hostPackCandidates } from "./rooms";

const inst = (id: string, mc: string, kind: Instance["loader"]["kind"], version: string | null, createdAt: string): Instance => ({
  id,
  name: id,
  mc,
  loader: { kind, version },
  java: { path: null, major: 21 },
  memory: { minMb: 1024, maxMb: 4096, auto: true },
  jvmArgs: [],
  preset: null,
  icon: null,
  createdAt,
  lastPlayed: null,
  playtimeSec: 0,
});

const sig: InstanceSig = { mc: "1.21.1", loader: "fabric", loaderVersion: "0.16.14", modsHash: "x" };

describe("hostPackCandidates", () => {
  it("keeps same game/loader/version, newest first", () => {
    const list = [
      inst("old", "1.21.1", "fabric", "0.16.14", "2025-01-01T00:00:00Z"),
      inst("other-ver", "1.21.1", "fabric", "0.15.0", "2026-01-01T00:00:00Z"),
      inst("forge", "1.21.1", "forge", "0.16.14", "2026-01-01T00:00:00Z"),
      inst("new", "1.21.1", "fabric", "0.16.14", "2026-05-01T00:00:00Z"),
      inst("mc", "1.20.1", "fabric", "0.16.14", "2026-05-01T00:00:00Z"),
    ];
    expect(hostPackCandidates(list, sig).map((i) => i.id)).toEqual(["new", "old"]);
    expect(hostPackCandidates(list, { ...sig, loaderVersion: null }).map((i) => i.id)).toEqual(["new", "other-ver", "old"]);
  });
});

describe("hostPackStore", () => {
  beforeEach(() => useHostPackStore.getState().reset());

  it("tracks one room; updates for other rooms are ignored", () => {
    const s = useHostPackStore.getState();
    expect(s.begin("r1")).toBe(true);
    expect(useHostPackStore.getState().begin("r1")).toBe(false);
    s.setPhase("r2", "needPack");
    expect(useHostPackStore.getState().phase).toBe("matching");
    s.attach("r1", "inst-a");
    expect(useHostPackStore.getState()).toMatchObject({ phase: "attached", instance: "inst-a" });
    expect(s.begin("r2")).toBe(true);
    expect(useHostPackStore.getState()).toMatchObject({ roomId: "r2", phase: "matching", instance: null });
  });
});
