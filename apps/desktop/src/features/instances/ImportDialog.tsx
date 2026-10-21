import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../api/client";
import { qk, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { IMPORT_EXTS, validateFilePath } from "../../lib/validation";

/** Imports a `.mrpack` or CurseForge zip by path; progress goes to the task list. */
export function ImportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [path, setPath] = useState("");
  const [touched, setTouched] = useState(false);
  const imp = useStartTask<string>({
    run: (p, ch) => api.instanceImport(p, ch),
    invalidate: [qk.instances],
  });

  const resetImp = imp.reset;
  useEffect(() => {
    if (open) {
      setPath("");
      setTouched(false);
      resetImp();
    }
  }, [open, resetImp]);

  const error = validateFilePath(path, IMPORT_EXTS);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("import.title")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" type="submit" form="import-instance" loading={imp.isPending} disabled={touched && !!error}>
            {t("import.submit")}
          </Button>
        </>
      }
    >
      <form
        id="import-instance"
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          setTouched(true);
          if (error) return;
          imp.mutate(path.trim(), { onSuccess: onClose });
        }}
      >
        <p className="text-sm text-muted">{t("import.hint")}</p>
        <Field label={t("import.path")} error={touched ? error : null} hint={t("import.pathHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              autoFocus
              aria-describedby={d}
              invalid={inv}
              value={path}
              placeholder={t("import.pathPlaceholder")}
              onChange={(e) => setPath(e.target.value)}
              onBlur={() => setTouched(true)}
            />
          )}
        </Field>
        <ErrorMessage error={imp.error} />
      </form>
    </Modal>
  );
}
