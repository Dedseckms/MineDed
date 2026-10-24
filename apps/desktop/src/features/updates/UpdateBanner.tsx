import { AnimatePresence, motion } from "motion/react";
import { useTranslation } from "react-i18next";

import type { UpdateInfo } from "../../bindings";
import { useAppVersion } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { ErrorMessage } from "../../components/ui/feedback";
import { useMotionPreset } from "../../hooks/useAppearance";
import { useInstallUpdate } from "../../hooks/useUpdates";
import { selectBannerUpdate, useUpdateStore } from "../../stores/updateStore";
import { TaskProgress } from "../tasks/TaskProgress";

/** Release notes as plain text (never HTML); long notes scroll. */
export function UpdateNotes({ notes }: { notes: string }) {
  const { t } = useTranslation();
  const text = notes.trim();
  if (!text) return null;
  return (
    <details className="text-xs text-muted">
      <summary className="cursor-pointer select-none hover:text-text">{t("updates.notes")}</summary>
      <p className="mt-1 max-h-40 overflow-y-auto whitespace-pre-line break-words">{text}</p>
    </details>
  );
}

/** Install button + live progress; shared by the banner and the settings section. */
export function InstallUpdateControls({ info, onLater }: { info: UpdateInfo; onLater?: () => void }) {
  const { t } = useTranslation();
  const { install, running, task, error } = useInstallUpdate();
  const failed = task?.status === "failed" || error !== null;
  return (
    <div className="flex flex-col gap-2">
      {task?.status === "running" ? <TaskProgress task={task} compact /> : null}
      {task?.status === "failed" && task.error ? <ErrorMessage error={task.error} /> : null}
      <ErrorMessage error={error} />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="primary"
          loading={running}
          disabled={running}
          onClick={install}
          icon={<Icon name="download" />}
          title={t("updates.installVersion", { version: info.version })}
        >
          {failed ? t("common.retry") : t("updates.install")}
        </Button>
        {onLater ? (
          <Button size="sm" variant="ghost" disabled={running} onClick={onLater}>
            {t("updates.later")}
          </Button>
        ) : null}
        <span className="text-xs text-muted">{t("updates.restartHint")}</span>
      </div>
    </div>
  );
}

/** Non-blocking strip under the header: "Version X is available". */
export function UpdateBanner() {
  const { t } = useTranslation();
  const info = useUpdateStore(selectBannerUpdate);
  const dismiss = useUpdateStore((s) => s.dismiss);
  const current = useAppVersion();
  const preset = useMotionPreset("item");
  return (
    <AnimatePresence initial={false}>
      {info ? (
        <motion.section
          key={info.version}
          {...preset}
          role="status"
          aria-label={t("updates.available", { version: info.version })}
          data-testid="update-banner"
          className="shrink-0 border-b border-accent/30 bg-accent/10 px-4 py-3 lg:px-6"
        >
          <div className="mx-auto flex w-full max-w-5xl flex-col gap-2 2xl:max-w-6xl">
            <div className="flex items-start gap-3">
              <span className="mt-0.5 text-accent">
                <Icon name="download" />
              </span>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold">{t("updates.available", { version: info.version })}</p>
                <p className="text-xs text-muted">
                  {t("updates.current", { version: info.currentVersion || current.data || "…" })}
                </p>
              </div>
            </div>
            <UpdateNotes notes={info.notes} />
            <InstallUpdateControls info={info} onLater={dismiss} />
          </div>
        </motion.section>
      ) : null}
    </AnimatePresence>
  );
}
