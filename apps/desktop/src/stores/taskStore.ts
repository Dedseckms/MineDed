import { create } from "zustand";

import type { TaskEvent, TaskSummary } from "../bindings";
import { type TaskView, applyTaskEvent, mergeSummary } from "../lib/tasks";

interface TaskState {
  tasks: Record<string, TaskView>;
  /**
   * Monotonic sequence number of the first time the store saw each task.
   * `TaskId` is a UUIDv4, so ids carry no ordering — this is the only clock.
   */
  firstSeen: Record<string, number>;
  nextSeq: number;
  /** Newest first (by `firstSeen`). */
  order: string[];
  /** Finished tasks hidden by the user (the backend keeps them). */
  dismissed: Record<string, true>;
  applyEvent: (ev: TaskEvent) => void;
  syncSummaries: (list: TaskSummary[]) => void;
  dismissFinished: () => void;
  reset: () => void;
}

type Clock = Pick<TaskState, "firstSeen" | "nextSeq" | "order">;

/** Registers `id` on first sight; returns the updated clock. */
function see(clock: Clock, id: string): Clock {
  if (id in clock.firstSeen) return clock;
  // New ids are always the newest, so prepending keeps `order` sorted by `firstSeen` desc.
  return {
    firstSeen: { ...clock.firstSeen, [id]: clock.nextSeq },
    nextSeq: clock.nextSeq + 1,
    order: [id, ...clock.order],
  };
}

const initial = { tasks: {}, firstSeen: {}, nextSeq: 0, order: [], dismissed: {} };

export const useTaskStore = create<TaskState>()((set) => ({
  ...initial,
  applyEvent: (ev) =>
    set((s) => ({
      ...see(s, ev.id),
      tasks: { ...s.tasks, [ev.id]: applyTaskEvent(s.tasks[ev.id], ev) },
    })),
  syncSummaries: (list) =>
    set((s) => {
      const tasks = { ...s.tasks };
      let clock: Clock = s;
      // `tasks_list` order is meaningless (sorted by UUID); unknown tasks get
      // a sequence number in arrival order, known ones keep their position.
      for (const summary of list) {
        tasks[summary.id] = mergeSummary(tasks[summary.id], summary);
        clock = see(clock, summary.id);
      }
      return { tasks, ...clock };
    }),
  dismissFinished: () =>
    set((s) => {
      const dismissed = { ...s.dismissed };
      for (const id of s.order) if (s.tasks[id]?.status !== "running") dismissed[id] = true;
      return { dismissed };
    }),
  reset: () => set(initial),
}));

/** Visible tasks, newest first. */
export function selectVisibleTasks(s: Pick<TaskState, "tasks" | "order" | "dismissed">): TaskView[] {
  const out: TaskView[] = [];
  for (const id of s.order) {
    const t = s.tasks[id];
    if (t && !s.dismissed[id]) out.push(t);
  }
  return out;
}

export function selectRunningCount(s: Pick<TaskState, "tasks">): number {
  let n = 0;
  for (const t of Object.values(s.tasks)) if (t.status === "running") n += 1;
  return n;
}
