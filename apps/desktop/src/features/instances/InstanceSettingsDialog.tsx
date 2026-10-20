import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Instance, MemoryCfg } from "../../bindings";
import { useRamRecommend, useSystemInfo, useUpdateInstance } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import {
  type MemoryErrors,
  formatArgs,
  hasErrors,
  parseArgs,
  pathOrNull,
  validateInstanceName,
  validateMemory,
  validateOptionalPath,
} from "../../lib/validation";
import { MemoryEditor, useFormatMb } from "../common/MemoryEditor";
import { useKeyLabel } from "../tasks/TaskProgress";

interface Draft {
  name: string;
  memory: MemoryCfg;
  javaPath: string;
  jvmArgs: string;
}

const toDraft = (i: Instance): Draft => ({
  name: i.name,
  memory: { ...i.memory },
  javaPath: i.java.path ?? "",
  jvmArgs: formatArgs(i.jvmArgs),
});

export function InstanceSettingsDialog({ instance, onClose }: { instance: Instance | null; onClose: () => void }) {
  const { t } = useTranslation();
  const label = useKeyLabel();
  const formatMb = useFormatMb();
  const [draft, setDraft] = useState<Draft | null>(null);
  const system = useSystemInfo();
  const recommend = useRamRecommend(instance?.id ?? null);
  const update = useUpdateInstance();

  // Re-seed the form only when another instance is opened (not on list refetch).
  const instanceRef = useRef(instance);
  instanceRef.current = instance;
  const resetUpdate = update.reset;
  const instanceId = instance?.id ?? null;
  useEffect(() => {
    const current = instanceRef.current;
    // Keep the last draft while the dialog animates out.
    if (current) {
      setDraft(toDraft(current));
      resetUpdate();
    }
  }, [instanceId, resetUpdate]);

  const totalRam = system.data?.totalRamMb ?? null;
  const errors: MemoryErrors & { name?: string | null; javaPath?: string | null } = draft
    ? {
        name: validateInstanceName(draft.name),
        javaPath: validateOptionalPath(draft.javaPath),
        ...validateMemory(draft.memory.minMb, draft.memory.maxMb, totalRam),
      }
    : {};
  const args = draft ? parseArgs(draft.jvmArgs) : null;
  const argsError = args && !args.ok ? args.error : null;
  const invalid = hasErrors(errors) || argsError !== null;

  const save = () => {
    if (!instance || !draft || invalid || !args?.ok) return;
    update.mutate(
      {
        id: instance.id,
        patch: {
          name: draft.name.trim(),
          java: { ...instance.java, path: pathOrNull(draft.javaPath) },
          memory: draft.memory,
          jvmArgs: args.args,
          icon: null,
        },
      },
      { onSuccess: onClose },
    );
  };

  const hint = recommend.data
    ? t("memory.recommended", {
        min: formatMb(recommend.data.minMb),
        max: formatMb(recommend.data.maxMb),
        reason: label(recommend.data.reasonKey, "memory.reasonGeneric"),
      })
    : totalRam
      ? t("memory.systemTotal", { total: formatMb(totalRam) })
      : null;

  return (
    <Modal
      open={instance !== null}
      onClose={onClose}
      wide
      title={t("instances.settingsTitle", { name: instance?.name ?? "" })}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" disabled={invalid} loading={update.isPending} onClick={save}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      {draft ? (
        <div className="flex flex-col gap-5">
          <Field label={t("instances.name")} error={errors.name}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              />
            )}
          </Field>
          <MemoryEditor
            value={draft.memory}
            onChange={(memory) => setDraft({ ...draft, memory })}
            totalRamMb={totalRam}
            errors={errors}
            hint={hint}
          />
          <Field label={t("instances.javaPath")} error={errors.javaPath} hint={t("instances.javaPathHint")}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                value={draft.javaPath}
                placeholder={t("instances.javaPathPlaceholder")}
                onChange={(e) => setDraft({ ...draft, javaPath: e.target.value })}
              />
            )}
          </Field>
          <Field label={t("instances.jvmArgs")} error={argsError} hint={t("instances.jvmArgsHint")}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                className="font-mono"
                value={draft.jvmArgs}
                placeholder="-XX:+UseG1GC"
                onChange={(e) => setDraft({ ...draft, jvmArgs: e.target.value })}
              />
            )}
          </Field>
          <ErrorMessage error={update.error} />
        </div>
      ) : null}
    </Modal>
  );
}
