import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Instance, InstanceId } from "../../bindings";
import { api } from "../../api/client";
import { qk, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { validateInstanceName } from "../../lib/validation";

export function CloneInstanceDialog({ source, onClose }: { source: Instance | null; onClose: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const clone = useStartTask<{ id: InstanceId; name: string }>({
    run: (v, ch) => api.instanceClone(v.id, v.name, ch),
    invalidate: [qk.instances],
  });

  const sourceName = source?.name;
  const resetClone = clone.reset;
  useEffect(() => {
    if (sourceName !== undefined) {
      setName(t("instances.cloneDefaultName", { name: sourceName }));
      resetClone();
    }
  }, [sourceName, resetClone, t]);

  const error = validateInstanceName(name);
  const submit = () => {
    if (!source || error) return;
    clone.mutate({ id: source.id, name: name.trim() }, { onSuccess: onClose });
  };

  return (
    <Modal
      open={source !== null}
      onClose={onClose}
      title={t("instances.cloneTitle", { name: source?.name ?? "" })}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" type="submit" form="clone-instance" disabled={!!error} loading={clone.isPending}>
            {t("instances.clone")}
          </Button>
        </>
      }
    >
      <form
        id="clone-instance"
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <Field label={t("instances.name")} error={error}>
          {(id, d, inv) => (
            <TextInput id={id} autoFocus aria-describedby={d} invalid={inv} value={name} onChange={(e) => setName(e.target.value)} />
          )}
        </Field>
        <ErrorMessage error={clone.error} />
      </form>
    </Modal>
  );
}
