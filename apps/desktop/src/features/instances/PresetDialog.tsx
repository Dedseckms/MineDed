import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Instance, PerfPreset } from "../../bindings";
import { usePresetApply } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { useTaskView } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

export const PRESETS: PerfPreset[] = ["potato", "balanced", "quality"];

export function PresetDialog({ instance, onClose }: { instance: Instance | null; onClose: () => void }) {
  const { t } = useTranslation();
  const [choice, setChoice] = useState<PerfPreset>("balanced");
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskView(taskId);
  // Re-reads the instance on finish, so the "current" badge follows the persisted preset.
  const apply = usePresetApply();

  const resetApply = apply.reset;
  const currentPreset = instance?.preset ?? null;
  const open = instance !== null;
  useEffect(() => {
    if (!open) return;
    setChoice(currentPreset ?? "balanced");
    setTaskId(null);
    resetApply();
  }, [open, resetApply]); // Only on open: later preset changes come from our own apply.

  const running = apply.isPending || task?.status === "running";
  const done = task?.status === "succeeded";

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("preset.title")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {done ? t("common.close") : t("common.cancel")}
          </Button>
          {!done ? (
            <Button
              variant="primary"
              loading={running}
              onClick={() => {
                if (instance) apply.mutate({ id: instance.id, preset: choice }, { onSuccess: setTaskId });
              }}
            >
              {t("preset.apply")}
            </Button>
          ) : null}
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <p className="text-sm text-muted">{t("preset.hint")}</p>
        <div role="radiogroup" aria-label={t("preset.title")} className="flex flex-col gap-2">
          {PRESETS.map((p) => (
            <button
              key={p}
              type="button"
              role="radio"
              aria-checked={choice === p}
              disabled={running}
              onClick={() => setChoice(p)}
              className={`flex cursor-pointer flex-col items-start gap-1 rounded-xl border p-3 text-left transition-colors disabled:cursor-not-allowed ${
                choice === p ? "border-accent bg-accent/10" : "border-border hover:bg-surface-2"
              }`}
            >
              <span className="flex items-center gap-2 font-medium">
                {t(`preset.${p}.title`)}
                {currentPreset === p ? <Badge tone="accent">{t("preset.current")}</Badge> : null}
              </span>
              <span className="text-xs text-muted">{t(`preset.${p}.description`)}</span>
            </button>
          ))}
        </div>
        {task ? <TaskProgress task={task} /> : null}
        {done ? <p className="text-sm text-muted">{t("preset.appliedHint")}</p> : null}
        <ErrorMessage error={apply.error} />
      </div>
    </Modal>
  );
}
