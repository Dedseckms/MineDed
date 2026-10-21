import { useTranslation } from "react-i18next";

import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { useCancelTask } from "../../api/queries";
import { type TaskView, formatSpeed, taskFraction } from "../../lib/tasks";

/** Translates a backend-provided i18n key, with a generic fallback. */
export function useKeyLabel() {
  const { t, i18n } = useTranslation();
  return (key: string, fallback: string) => (key && i18n.exists(key) ? t(key) : t(fallback));
}

const STATUS_TONE = {
  running: "info",
  succeeded: "accent",
  failed: "danger",
  cancelled: "neutral",
} as const;

export function TaskProgress({ task, compact = false }: { task: TaskView; compact?: boolean }) {
  const { t } = useTranslation();
  const label = useKeyLabel();
  const cancel = useCancelTask();
  const fraction = taskFraction(task);
  const speed = task.speedBps !== null && task.status === "running" ? formatSpeed(task.speedBps) : null;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          {!compact ? (
            <p className="truncate text-sm font-medium">{label(task.titleKey, "tasks.generic")}</p>
          ) : null}
          <p className="truncate text-xs text-muted">
            {task.status === "running"
              ? label(task.stage, "stages.working")
              : t(`taskStatus.${task.status}`)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2 text-xs text-muted tabular-nums">
          {speed ? <span>{t("tasks.speed", { value: speed.value, unit: t(speed.unitKey) })}</span> : null}
          {fraction !== null && task.status === "running" ? <span>{Math.round(fraction * 100)}%</span> : null}
          {task.status !== "running" ? (
            <Badge tone={STATUS_TONE[task.status]}>{t(`taskStatus.${task.status}`)}</Badge>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              loading={cancel.isPending}
              onClick={() => cancel.mutate(task.id)}
              aria-label={t("tasks.cancel")}
            >
              {t("tasks.cancel")}
            </Button>
          )}
        </div>
      </div>
      {task.status === "running" ? (
        <ProgressBar value={fraction} label={label(task.titleKey, "tasks.generic")} />
      ) : null}
      {task.status === "failed" && task.error ? <ErrorMessage error={task.error} /> : null}
      {cancel.error ? <ErrorMessage error={cancel.error} /> : null}
    </div>
  );
}
