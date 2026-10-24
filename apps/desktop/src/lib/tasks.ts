// Pure reducers that turn `TaskEvent`s / `TaskSummary`s into a view model.
import type { AppError, TaskEvent, TaskKind, TaskStatus, TaskSummary } from "../bindings";

/** Max log lines kept per task (the task center shows only the tail). */
export const TASK_LOG_LIMIT = 50;

export interface TaskView {
  id: string;
  kind: TaskKind | null;
  titleKey: string;
  status: TaskStatus;
  done: number;
  total: number | null;
  /** i18n key of the current stage (may be empty). */
  stage: string;
  speedBps: number | null;
  error: AppError | null;
  log: string[];
  /** `true` when this window owns the task's channel (fresher than summaries). */
  live: boolean;
}

function blank(id: string): TaskView {
  return {
    id,
    kind: null,
    titleKey: "",
    status: "running",
    done: 0,
    total: null,
    stage: "",
    speedBps: null,
    error: null,
    log: [],
    live: true,
  };
}

export function statusFromError(error: AppError | null): TaskStatus {
  if (!error) return "succeeded";
  return error.code === "cancelled" ? "cancelled" : "failed";
}

/** Applies one channel event. Events after `finished` are ignored. */
export function applyTaskEvent(prev: TaskView | undefined, ev: TaskEvent): TaskView {
  const base = prev ?? blank(ev.id);
  if (base.status !== "running" && ev.type !== "started") return base;
  switch (ev.type) {
    case "started":
      return { ...base, kind: ev.kind, titleKey: ev.titleKey, status: "running", live: true };
    case "progress":
      return {
        ...base,
        done: ev.done,
        total: ev.total,
        stage: ev.stage,
        speedBps: ev.speedBps,
        live: true,
      };
    case "log": {
      const log = [...base.log, ev.line];
      return { ...base, log: log.length > TASK_LOG_LIMIT ? log.slice(-TASK_LOG_LIMIT) : log };
    }
    case "finished":
      return { ...base, status: statusFromError(ev.error), error: ev.error, speedBps: null };
  }
}

/**
 * Merges a row from `tasks_list` / `tasks://changed`. Running tasks fed by our
 * own channel keep their (fresher) progress; terminal statuses always win.
 */
export function mergeSummary(prev: TaskView | undefined, s: TaskSummary): TaskView {
  const base = prev ?? { ...blank(s.id), live: false };
  const next: TaskView = { ...base, kind: s.kind, titleKey: s.titleKey || base.titleKey };
  if (base.status !== "running") return next; // already final locally
  if (s.status !== "running") {
    return { ...next, status: s.status, done: s.done, total: s.total, speedBps: null };
  }
  if (base.live) return next;
  return { ...next, done: s.done, total: s.total, stage: s.stage };
}

/** Progress in 0..1, or `null` when the total is unknown (indeterminate bar). */
export function progressFraction(done: number, total: number | null): number | null {
  if (total === null || total <= 0) return null;
  return Math.min(1, Math.max(0, done / total));
}

export function taskFraction(t: Pick<TaskView, "status" | "done" | "total">): number | null {
  if (t.status === "succeeded") return 1;
  return progressFraction(t.done, t.total);
}

/**
 * i18n key of a progress stage. Most backends send full keys (`stages.download`);
 * some send bare names (`download`, e.g. the self-updater), which map to `stages.<name>`.
 */
export function stageKey(stage: string, exists: (key: string) => boolean): string {
  if (!stage || exists(stage)) return stage;
  const scoped = `stages.${stage}`;
  return exists(scoped) ? scoped : stage;
}

const UNITS = ["units.bps", "units.kbps", "units.mbps", "units.gbps"] as const;

/** Human speed: value string + i18n key of the unit. */
export function formatSpeed(bps: number): { value: string; unitKey: (typeof UNITS)[number] } {
  let v = Math.max(0, bps);
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i += 1;
  }
  const value = i === 0 || v >= 100 ? Math.round(v).toString() : v.toFixed(1);
  return { value, unitKey: UNITS[i] ?? "units.bps" };
}
