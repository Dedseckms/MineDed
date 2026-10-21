import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { LoaderKind, VersionKind } from "../../bindings";
import { api } from "../../api/client";
import { qk, useStartTask, useVersions } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, Segmented, Select, TextInput } from "../../components/ui/form";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { validateInstanceName } from "../../lib/validation";

const LOADERS: LoaderKind[] = ["vanilla", "fabric", "quilt", "forge", "neoForge"];
type KindFilter = Extract<VersionKind, "release" | "snapshot">;

export function CreateInstanceDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [kind, setKind] = useState<KindFilter>("release");
  const [mc, setMc] = useState("");
  const [touched, setTouched] = useState(false);
  const versions = useVersions([kind]);

  const create = useStartTask<{ name: string; mc: string }>({
    run: (v, ch) =>
      api.instanceCreate(
        { name: v.name, mc: v.mc, loader: { kind: "vanilla", version: null }, preset: null, icon: null },
        ch,
      ),
    invalidate: [qk.instances],
  });

  // Default to the newest version of the selected kind.
  useEffect(() => {
    const list = versions.data ?? [];
    if (!list.some((v) => v.id === mc)) setMc(list[0]?.id ?? "");
  }, [versions.data, mc]);

  const resetCreate = create.reset;
  useEffect(() => {
    if (open) {
      setName("");
      setTouched(false);
      resetCreate();
    }
  }, [open, resetCreate]);

  const nameError = validateInstanceName(name);
  const canSubmit = nameError === null && mc !== "" && !create.isPending;

  const submit = () => {
    setTouched(true);
    if (!canSubmit) return;
    create.mutate({ name: name.trim(), mc }, { onSuccess: onClose });
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("instances.createTitle")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" type="submit" form="create-instance" disabled={!canSubmit && touched} loading={create.isPending}>
            {t("instances.create")}
          </Button>
        </>
      }
    >
      <form
        id="create-instance"
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <Field label={t("instances.name")} error={touched ? nameError : null}>
          {(id, describedBy, invalid) => (
            <TextInput
              id={id}
              autoFocus
              value={name}
              invalid={invalid}
              aria-describedby={describedBy}
              placeholder={t("instances.namePlaceholder")}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => setTouched(true)}
            />
          )}
        </Field>

        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("instances.version")}</span>
          <Segmented<KindFilter>
            ariaLabel={t("instances.versionKind")}
            value={kind}
            onChange={setKind}
            options={[
              { value: "release", label: t("versionKind.release") },
              { value: "snapshot", label: t("versionKind.snapshot") },
            ]}
          />
        </div>
        <Field label={t("instances.mcVersion")}>
          {(id) => (
            <Select
              id={id}
              value={mc}
              disabled={versions.isPending || !versions.data?.length}
              onChange={(e) => setMc(e.target.value)}
            >
              {versions.isPending ? <option value="">{t("common.loading")}</option> : null}
              {(versions.data ?? []).map((v) => (
                <option key={v.id} value={v.id}>
                  {v.id}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <ErrorMessage error={versions.error} />

        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("instances.loader")}</span>
          <Segmented<LoaderKind>
            ariaLabel={t("instances.loader")}
            value="vanilla"
            onChange={() => {}}
            options={LOADERS.map((l) => ({
              value: l,
              label: t(`loader.${l}`),
              disabled: l !== "vanilla",
              badge: l !== "vanilla" ? <Badge>{t("common.soon")}</Badge> : undefined,
            }))}
          />
        </div>
        <ErrorMessage error={create.error} />
      </form>
    </Modal>
  );
}
