import { create } from "zustand";

import type {
  InstanceId,
  InstanceLogPayload,
  InstanceStatePayload,
  RunState,
} from "../bindings";

/** Lines of game log kept per instance (the Play screen shows the tail). */
export const GAME_LOG_LIMIT = 500;

export interface RunInfo {
  state: RunState;
  exitCode: number | null;
  crashReport: string | null;
}

interface GameState {
  runs: Record<InstanceId, RunInfo>;
  logs: Record<InstanceId, string[]>;
  /** Launch task per instance (to show its progress on the Play screen). */
  launchTasks: Record<InstanceId, string>;
  applyState: (p: InstanceStatePayload) => void;
  setState: (id: InstanceId, state: RunState) => void;
  appendLog: (p: InstanceLogPayload) => void;
  clearLog: (id: InstanceId) => void;
  setLaunchTask: (id: InstanceId, taskId: string) => void;
  reset: () => void;
}

export const useGameStore = create<GameState>()((set) => ({
  runs: {},
  logs: {},
  launchTasks: {},
  applyState: (p) =>
    set((s) => ({
      runs: { ...s.runs, [p.id]: { state: p.state, exitCode: p.exitCode, crashReport: p.crashReport } },
    })),
  setState: (id, state) =>
    set((s) => {
      // Only fills in unknown state: events are always fresher than queries.
      if (s.runs[id]) return s;
      return { runs: { ...s.runs, [id]: { state, exitCode: null, crashReport: null } } };
    }),
  appendLog: (p) =>
    set((s) => {
      if (p.lines.length === 0) return s;
      const merged = [...(s.logs[p.id] ?? []), ...p.lines];
      const lines = merged.length > GAME_LOG_LIMIT ? merged.slice(-GAME_LOG_LIMIT) : merged;
      return { logs: { ...s.logs, [p.id]: lines } };
    }),
  clearLog: (id) => set((s) => ({ logs: { ...s.logs, [id]: [] } })),
  setLaunchTask: (id, taskId) => set((s) => ({ launchTasks: { ...s.launchTasks, [id]: taskId } })),
  reset: () => set({ runs: {}, logs: {}, launchTasks: {} }),
}));

export const isActive = (state: RunState | undefined): boolean =>
  state === "running" || state === "preparing";
