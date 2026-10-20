import { create } from "zustand";

import type { TaskEvent, TaskSummary } from "../bindings";
import { type TaskView, applyTaskEvent, mergeSummary } from "../lib/tasks";

interface TaskState {
  tasks: Record<string, TaskView>;
  /** Newest first. */
  order: string[];
  /** Finished tasks hidden by the user (the backend keeps them). */
  dismissed: Record<string, true>;
  applyEvent: (ev: TaskEvent) => void;
  syncSummaries: (list: TaskSummary[]) => void;
  dismissFinished: () => void;
  reset: () => void;
}

const withId = (order: string[], id: string) => (order.includes(id) ? order : [id, ...order]);

export const useTaskStore = create<TaskState>()((set) => ({
  tasks: {},
  order: [],
  dismissed: {},
  applyEvent: (ev) =>
    set((s) => ({
      tasks: { ...s.tasks, [ev.id]: applyTaskEvent(s.tasks[ev.id], ev) },
      order: withId(s.order, ev.id),
    })),
  syncSummaries: (list) =>
    set((s) => {
      const tasks = { ...s.tasks };
      let order = s.order;
      // Summaries come sorted by id ascending; prepend in that order → newest first.
      for (const summary of list) {
        tasks[summary.id] = mergeSummary(tasks[summary.id], summary);
        order = withId(order, summary.id);
      }
      return { tasks, order };
    }),
  dismissFinished: () =>
    set((s) => {
      const dismissed = { ...s.dismissed };
      for (const id of s.order) if (s.tasks[id]?.status !== "running") dismissed[id] = true;
      return { dismissed };
    }),
  reset: () => set({ tasks: {}, order: [], dismissed: {} }),
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
