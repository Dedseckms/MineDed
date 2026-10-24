import { useTranslation } from "react-i18next";

import type { Settings } from "../../bindings";
import { isDemo, isMock } from "../../api/client";
import { useAppVersion, useSaveSettings } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Section } from "../../components/ui/Card";
import { Switch } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { useCheckUpdate } from "../../hooks/useUpdates";
import { useUpdateStore } from "../../stores/updateStore";
import { InstallUpdateControls, UpdateNotes } from "./UpdateBanner";

/** Settings → "About & updates": version, auto-check switch, manual check. */
export function AboutSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation();
  const version = useAppVersion();
  const save = useSaveSettings();
  const check = useCheckUpdate();
  const available = useUpdateStore((s) => s.available);

  return (
    <Section title={t("updates.section")}>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted">{t("updates.version")}</dt>
        <dd className="font-medium tabular-nums" data-testid="app-version">
          {version.data ?? "…"}
          {isMock && !isDemo ? <span className="ml-2 text-xs font-normal text-muted">{t("app.mock")}</span> : null}
        </dd>
      </dl>
      <Switch
        checked={settings.autoUpdate}
        onChange={(autoUpdate) => save.mutate({ ...settings, autoUpdate })}
        label={t("updates.auto")}
        description={t("updates.autoHint")}
      />
      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="secondary"
            loading={check.isPending}
            disabled={check.isPending}
            onClick={() => check.mutate()}
            icon={<Icon name="refresh" />}
          >
            {check.isPending ? t("updates.checking") : t("updates.check")}
          </Button>
          {check.isSuccess && check.data === null ? (
            <p role="status" className="flex items-center gap-1.5 text-sm text-accent">
              <Icon name="check" />
              {t("updates.upToDate")}
            </p>
          ) : null}
        </div>
        {check.isError ? (
          <div className="flex flex-col gap-1">
            <p className="text-sm font-medium text-danger">{t("updates.checkFailed")}</p>
            <ErrorMessage error={check.error} />
          </div>
        ) : null}
        {available ? (
          <div className="flex flex-col gap-2 rounded-lg border border-accent/30 bg-accent/10 p-3">
            <p className="text-sm font-semibold">{t("updates.available", { version: available.version })}</p>
            <UpdateNotes notes={available.notes} />
            <InstallUpdateControls info={available} />
          </div>
        ) : null}
      </div>
      <ErrorMessage error={save.error} />
    </Section>
  );
}
