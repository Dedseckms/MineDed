import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { LoaderKind, PerfPreset, VersionKind } from "../../bindings";
import { useCreateInstance, useLoaderVersions, useVersions } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, Segmented, Select, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { pickLoaderVersion } from "../../lib/content";
import { validateInstanceName } from "../../lib/validation";
import { PRESETS } from "./PresetDialog";

const LOADERS: LoaderKind[] = ["vanilla", "fabric", "quilt", "forge", "neoForge"];
type KindFilter = Extract<VersionKind, "release" | "snapshot">;

export function CreateInstanceDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [kind, setKind] = useState<KindFilter>("release");
  const [mc, setMc] = useState("");
  const [loader, setLoader] = useState<LoaderKind>("vanilla");
  const [loaderVersion, setLoaderVersion] = useState("");
  const [preset, setPreset] = useState<PerfPreset | "">("");
  const [touched, setTouched] = useState(false);
  const versions = useVersions([kind]);
  const loaderVersions = useLoaderVersions(loader, mc);

  const create = useCreateInstance();
  const startCreate = create.create;

  // Default to the newest version of the selected kind.
  useEffect(() => {
    const list = versions.data ?? [];
    if (!list.some((v) => v.id === mc)) setMc(list[0]?.id ?? "");
  }, [versions.data, mc]);

  // Default loader build: recommended → stable → newest.
  useEffect(() => {
    const list = loaderVersions.data ?? [];
    if (!list.some((v) => v.version === loaderVersion)) setLoaderVersion(pickLoaderVersion(list) ?? "");
  }, [loaderVersions.data, loaderVersion]);

  const resetCreate = create.reset;
  useEffect(() => {
    if (open) {
      setName("");
      setTouched(false);
      setLoader("vanilla");
      setPreset("");
      resetCreate();
    }
  }, [open, resetCreate]);

  const nameError = validateInstanceName(name);
  const needsLoaderVersion = loader !== "vanilla";
  const noLoaderBuilds = needsLoaderVersion && loaderVersions.isSuccess && loaderVersions.data.length === 0;
  const loaderReady = !needsLoaderVersion || loaderVersion !== "";
  const canSubmit = nameError === null && mc !== "" && loaderReady && !create.isPending;

  const submit = () => {
    setTouched(true);
    if (!canSubmit) return;
    startCreate({
      spec: {
        name: name.trim(),
        mc,
        loader: { kind: loader, version: needsLoaderVersion ? loaderVersion : null },
        preset: null,
        icon: null,
      },
      preset: preset === "" ? null : preset,
    }).then(onClose, () => {}); // The error is shown via `create.error`.
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
          <Button
            variant="primary"
            type="submit"
            form="create-instance"
            disabled={(!canSubmit && touched) || noLoaderBuilds}
            loading={create.isPending}
          >
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
            value={loader}
            onChange={setLoader}
            options={LOADERS.map((l) => ({ value: l, label: t(`loader.${l}`) }))}
          />
        </div>
        {needsLoaderVersion ? (
          <Field
            label={t("instances.loaderVersion")}
            hint={noLoaderBuilds ? null : t("instances.loaderVersionHint")}
            error={noLoaderBuilds ? "instances.noLoaderBuilds" : null}
          >
            {(id, d, inv) => (
              <Select
                id={id}
                aria-describedby={d}
                invalid={inv}
                value={loaderVersion}
                disabled={loaderVersions.isPending || noLoaderBuilds}
                onChange={(e) => setLoaderVersion(e.target.value)}
              >
                {loaderVersions.isPending ? <option value="">{t("common.loading")}</option> : null}
                {(loaderVersions.data ?? []).map((v) => (
                  <option key={v.version} value={v.version}>
                    {v.version}
                    {v.recommended ? ` · ${t("instances.recommended")}` : !v.stable ? ` · ${t("instances.unstable")}` : ""}
                  </option>
                ))}
              </Select>
            )}
          </Field>
        ) : null}
        <ErrorMessage error={loaderVersions.error} />

        <Field label={t("preset.optional")} hint={preset ? t(`preset.${preset}.description`) : t("preset.noneHint")}>
          {(id, d) => (
            <Select id={id} aria-describedby={d} value={preset} onChange={(e) => setPreset(e.target.value as PerfPreset | "")}>
              <option value="">{t("preset.none")}</option>
              {PRESETS.map((p) => (
                <option key={p} value={p}>
                  {t(`preset.${p}.title`)}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <ErrorMessage error={create.error} />
      </form>
    </Modal>
  );
}
