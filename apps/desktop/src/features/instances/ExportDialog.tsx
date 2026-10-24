import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Instance, PackFormat } from "../../bindings";
import { api } from "../../api/client";
import { useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { PACK_FORMATS, packExt, withPackExt } from "../../lib/content";
import { validateFilePath } from "../../lib/validation";
import { useTaskView } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

/** Only Modrinth `.mrpack` export is offered (CurseForge is turned off). */
const FORMAT: PackFormat = PACK_FORMATS[0];

export function ExportDialog({ instance, onClose }: { instance: Instance | null; onClose: () => void }) {
  const { t } = useTranslation();
  const format = FORMAT;
  const [path, setPath] = useState("");
  const [touched, setTouched] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskView(taskId);
  const exp = useStartTask<{ id: string; format: PackFormat; path: string }>({
    run: (v, ch) => api.instanceExport(v.id, v.format, v.path, ch),
  });

  const open = instance !== null;
  const resetExp = exp.reset;
  useEffect(() => {
    if (!open) return;
    setPath("");
    setTouched(false);
    setTaskId(null);
    resetExp();
  }, [open, resetExp]);

  const error = validateFilePath(path, [packExt(format)]);
  const running = exp.isPending || task?.status === "running";
  const done = task?.status === "succeeded";

  const submit = () => {
    setTouched(true);
    if (error || !instance) return;
    exp.mutate({ id: instance.id, format, path: path.trim() }, { onSuccess: setTaskId });
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("export.title", { name: instance?.name ?? "" })}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {done ? t("common.close") : t("common.cancel")}
          </Button>
          {!done ? (
            <Button variant="primary" type="submit" form="export-instance" loading={running} disabled={touched && !!error}>
              {t("instances.export")}
            </Button>
          ) : null}
        </>
      }
    >
      <form
        id="export-instance"
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <p className="text-sm text-muted">{t("export.hint")}</p>
        <Field label={t("export.path")} error={touched ? error : null} hint={t("export.pathHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={path}
              placeholder={t("export.pathPlaceholder")}
              onChange={(e) => setPath(e.target.value)}
              onBlur={() => {
                setTouched(true);
                if (path.trim() !== "" && !/\.mrpack$/i.test(path.trim())) setPath(withPackExt(path, FORMAT));
              }}
            />
          )}
        </Field>
        {task ? <TaskProgress task={task} /> : null}
        {done ? <p className="text-sm text-accent">{t("export.done", { path: path.trim() })}</p> : null}
        <ErrorMessage error={exp.error} />
      </form>
    </Modal>
  );
}
