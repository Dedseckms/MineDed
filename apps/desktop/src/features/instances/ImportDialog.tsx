import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../api/client";
import { pickFile } from "../../api/dialog";
import { qk, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { ErrorMessage } from "../../components/ui/feedback";
import { fileName } from "../../lib/path";
import { IMPORT_EXTS, validateFilePath } from "../../lib/validation";

const PICK_EXTS = IMPORT_EXTS.map((e) => e.slice(1));

/** Imports a modpack (`.mrpack` / `.zip`) picked from disk; progress goes to the task list. */
export function ImportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [path, setPath] = useState<string | null>(null);
  const [pickError, setPickError] = useState<unknown>(null);
  const imp = useStartTask<string>({
    run: (p, ch) => api.instanceImport(p, ch),
    invalidate: [qk.instances],
  });

  const resetImp = imp.reset;
  useEffect(() => {
    if (open) {
      setPath(null);
      setPickError(null);
      resetImp();
    }
  }, [open, resetImp]);

  const error = path ? validateFilePath(path, IMPORT_EXTS) : null;

  const pick = async () => {
    setPickError(null);
    imp.reset();
    try {
      const p = await pickFile([{ name: t("dialog.modpack"), extensions: PICK_EXTS }]);
      if (p) setPath(p);
    } catch (e) {
      setPickError(e);
    }
  };

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
          <Button
            variant="primary"
            loading={imp.isPending}
            disabled={!path || !!error}
            onClick={() => {
              if (path && !error) imp.mutate(path, { onSuccess: onClose });
            }}
          >
            {t("import.submit")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <p className="text-sm text-muted">{t("import.hint")}</p>
        <div className="flex min-w-0 items-center gap-3">
          <Button icon={<Icon name="folder" />} onClick={() => void pick()}>
            {t("import.pickFile")}
          </Button>
          <p
            className={`min-w-0 flex-1 truncate text-sm ${path ? "font-mono" : "text-muted"}`}
            title={path ?? undefined}
            data-testid="import-file"
          >
            {path ? fileName(path) : t("import.noFile")}
          </p>
        </div>
        {error ? <p className="text-xs text-danger">{t(error)}</p> : null}
        <ErrorMessage error={imp.error ?? pickError} />
      </div>
    </Modal>
  );
}
