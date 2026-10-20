import { describe, expect, it } from "vitest";

import type { TaskEvent, TaskSummary } from "../bindings";
import { TASK_LOG_LIMIT, applyTaskEvent, formatSpeed, mergeSummary, progressFraction, taskFraction } from "./tasks";

const run = (events: TaskEvent[]) => events.reduce(applyTaskEvent, undefined as ReturnType<typeof applyTaskEvent> | undefined);

describe("TaskEvent → progress", () => {
  it("tracks started/progress/finished", () => {
    const t = run([
      { type: "started", id: "t1", kind: "instanceCreate", titleKey: "tasks.instanceCreate" },
      { type: "progress", id: "t1", done: 25, total: 100, stage: "stages.libraries", speedBps: 2048 },
    ]);
    expect(t).toMatchObject({
      id: "t1",
      kind: "instanceCreate",
      titleKey: "tasks.instanceCreate",
      status: "running",
      stage: "stages.libraries",
      speedBps: 2048,
      live: true,
    });
    expect(t && taskFraction(t)).toBe(0.25);
    const done = applyTaskEvent(t, { type: "finished", id: "t1", error: null });
    expect(done.status).toBe("succeeded");
    expect(done.speedBps).toBeNull();
    expect(taskFraction(done)).toBe(1);
  });

  it("maps finished errors to failed / cancelled", () => {
    const err = { code: "network" as const, message: "x", details: null };
    expect(run([{ type: "finished", id: "a", error: err }])?.status).toBe("failed");
    expect(run([{ type: "finished", id: "a", error: { ...err, code: "cancelled" } }])?.status).toBe("cancelled");
    expect(run([{ type: "finished", id: "a", error: err }])?.error).toEqual(err);
  });

  it("ignores events after finished", () => {
    const t = run([
      { type: "finished", id: "a", error: null },
      { type: "progress", id: "a", done: 1, total: 2, stage: "s", speedBps: null },
    ]);
    expect(t?.status).toBe("succeeded");
    expect(t?.done).toBe(0);
  });

  it("works when progress arrives before started", () => {
    const t = run([
      { type: "progress", id: "a", done: 5, total: null, stage: "stages.manifest", speedBps: null },
      { type: "started", id: "a", kind: "launch", titleKey: "tasks.launch" },
    ]);
    expect(t).toMatchObject({ kind: "launch", done: 5, total: null });
    expect(t && taskFraction(t)).toBeNull();
  });

  it("keeps a bounded log tail", () => {
    const events: TaskEvent[] = Array.from({ length: TASK_LOG_LIMIT + 10 }, (_, i) => ({
      type: "log",
      id: "a",
      line: `l${i}`,
    }));
    const t = run(events);
    expect(t?.log).toHaveLength(TASK_LOG_LIMIT);
    expect(t?.log.at(-1)).toBe(`l${TASK_LOG_LIMIT + 9}`);
  });

  it("progressFraction clamps and handles unknown totals", () => {
    expect(progressFraction(5, null)).toBeNull();
    expect(progressFraction(5, 0)).toBeNull();
    expect(progressFraction(150, 100)).toBe(1);
    expect(progressFraction(-1, 100)).toBe(0);
  });
});

describe("mergeSummary", () => {
  const summary = (p: Partial<TaskSummary>): TaskSummary => ({
    id: "a",
    kind: "launch",
    titleKey: "tasks.launch",
    status: "running",
    done: 1,
    total: 10,
    stage: "stages.assets",
    ...p,
  });

  it("creates a view for foreign tasks", () => {
    expect(mergeSummary(undefined, summary({}))).toMatchObject({ live: false, done: 1, total: 10, stage: "stages.assets" });
  });

  it("does not regress live channel progress", () => {
    const live = run([{ type: "progress", id: "a", done: 8, total: 10, stage: "stages.client", speedBps: 1 }]);
    const merged = mergeSummary(live, summary({ done: 2 }));
    expect(merged.done).toBe(8);
    expect(merged.stage).toBe("stages.client");
  });

  it("terminal summary wins over running local state", () => {
    const live = run([{ type: "progress", id: "a", done: 8, total: 10, stage: "s", speedBps: 1 }]);
    expect(mergeSummary(live, summary({ status: "failed" })).status).toBe("failed");
  });

  it("final local state is kept", () => {
    const done = run([{ type: "finished", id: "a", error: null }]);
    expect(mergeSummary(done, summary({ status: "running" })).status).toBe("succeeded");
  });
});

describe("formatSpeed", () => {
  it.each([
    [0, "0", "units.bps"],
    [512, "512", "units.bps"],
    [1536, "1.5", "units.kbps"],
    [5 * 1024 * 1024, "5.0", "units.mbps"],
    [300 * 1024 * 1024, "300", "units.mbps"],
    [3 * 1024 ** 3, "3.0", "units.gbps"],
  ])("%d B/s", (bps, value, unitKey) => {
    expect(formatSpeed(bps)).toEqual({ value, unitKey });
  });
});
