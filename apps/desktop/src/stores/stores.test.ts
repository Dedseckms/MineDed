import { beforeEach, describe, expect, it } from "vitest";

import { GAME_LOG_LIMIT, isActive, useGameStore } from "./gameStore";
import { useNavStore } from "./navStore";
import { selectRunningCount, selectVisibleTasks, useTaskStore } from "./taskStore";

describe("taskStore", () => {
  beforeEach(() => useTaskStore.getState().reset());

  it("applies channel events, newest first", () => {
    const s = useTaskStore.getState();
    s.applyEvent({ type: "started", id: "1", kind: "launch", titleKey: "tasks.launch" });
    s.applyEvent({ type: "started", id: "2", kind: "instanceCreate", titleKey: "tasks.instanceCreate" });
    s.applyEvent({ type: "progress", id: "1", done: 3, total: 6, stage: "stages.assets", speedBps: 10 });
    const st = useTaskStore.getState();
    expect(st.order).toEqual(["2", "1"]);
    expect(st.tasks["1"]?.done).toBe(3);
    expect(selectRunningCount(st)).toBe(2);
  });

  it("syncs summaries and dismisses finished tasks", () => {
    const s = useTaskStore.getState();
    s.syncSummaries([
      { id: "a", kind: "javaInstall", titleKey: "tasks.javaInstall", status: "succeeded", done: 1, total: 1, stage: "" },
      { id: "b", kind: "launch", titleKey: "tasks.launch", status: "running", done: 0, total: null, stage: "" },
    ]);
    expect(selectVisibleTasks(useTaskStore.getState()).map((t) => t.id)).toEqual(["b", "a"]);
    useTaskStore.getState().dismissFinished();
    expect(selectVisibleTasks(useTaskStore.getState()).map((t) => t.id)).toEqual(["b"]);
    expect(selectRunningCount(useTaskStore.getState())).toBe(1);
  });
});

describe("gameStore", () => {
  beforeEach(() => useGameStore.getState().reset());

  it("tracks run state; events win over queried state", () => {
    const s = useGameStore.getState();
    s.applyState({ id: "i", state: "running", exitCode: null, crashReport: null });
    s.setState("i", "stopped");
    expect(useGameStore.getState().runs.i?.state).toBe("running");
    s.setState("j", "stopped");
    expect(useGameStore.getState().runs.j?.state).toBe("stopped");
    expect(isActive("preparing")).toBe(true);
    expect(isActive("crashed")).toBe(false);
  });

  it("keeps a bounded log", () => {
    const s = useGameStore.getState();
    s.appendLog({ id: "i", lines: Array.from({ length: GAME_LOG_LIMIT }, (_, n) => `a${n}`) });
    s.appendLog({ id: "i", lines: ["last"] });
    const lines = useGameStore.getState().logs.i ?? [];
    expect(lines).toHaveLength(GAME_LOG_LIMIT);
    expect(lines.at(-1)).toBe("last");
    s.clearLog("i");
    expect(useGameStore.getState().logs.i).toEqual([]);
  });
});

describe("navStore", () => {
  it("navigates and selects", () => {
    useNavStore.getState().go("settings");
    useNavStore.getState().selectPlayInstance("x");
    expect(useNavStore.getState()).toMatchObject({ section: "settings", playInstance: "x" });
  });
});
