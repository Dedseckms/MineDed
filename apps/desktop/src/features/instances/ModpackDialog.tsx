import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ProjectSummary } from "../../bindings";
import type { ModpackInstallVars } from "../../api/queries";
import { useContentVersions } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { Switch } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { formatCount } from "../../lib/content";
import { type ModpackFilters, modpackVersionFilter } from "../../lib/modpacks";
import type { TaskView } from "../../lib/tasks";
import { useFormatBytes } from "../common/ExternalLink";
import { ManualDownloadLinks } from "../common/ManualDownloadLinks";
import { ProjectIcon } from "../content/ProjectIcon";
import { TaskProgress } from "../tasks/TaskProgress";

/** Modpack card with its versions; installs the latest or a picked version as a new instance. */
export function ModpackDialog({
  project,
  filters,
  task,
  starting,
  startError,
  onInstall,
  onClose,
}: {
  project: ProjectSummary | null;
  filters: Pick<ModpackFilters, "mc" | "loader">;
  /** The install task of this project, if one was started from the catalog. */
  task: TaskView | undefined;
  /** `modpack_install` is being called. */
  starting: boolean;
  /** Error of the `modpack_install` call itself (task errors come in `task`). */
  startError: unknown;
  onInstall: (v: ModpackInstallVars) => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const formatBytes = useFormatBytes();
  const [all, setAll] = useState(false);
  const filtered = filters.mc !== null || filters.loader !== null;
  const versions = useContentVersions(
    project?.source ?? "modrinth",
    project?.projectId ?? null,
    modpackVersionFilter(filters, all || !filtered),
  );
  const busy = starting || task?.status === "running";
  const install = (versionId: string | null) => {
    if (project) onInstall({ source: project.source, projectId: project.projectId, versionId });
  };

  return (
    <Modal
      open={project !== null}
      onClose={onClose}
      wide
      title={project?.title ?? ""}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.close")}
          </Button>
          <Button
            variant="primary"
            icon={<Icon name="download" />}
            loading={starting}
            disabled={busy}
            onClick={() => install(null)}
          >
            {t("modpacks.installLatest")}
          </Button>
        </>
      }
    >
      {project ? (
        <div className="flex flex-col gap-4">
          <div className="flex gap-3">
            <ProjectIcon project={project} size="lg" />
            <div className="min-w-0 flex-1">
              <p className="text-sm">{project.description}</p>
              <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                <span>{t("content.by", { author: project.author })}</span>
                <span>· {t("content.downloads", { count: formatCount(project.downloads, i18n.language) })}</span>
                {project.categories.map((c) => (
                  <Badge key={c}>{c}</Badge>
                ))}
              </p>
            </div>
          </div>

          {task ? (
            <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-4" aria-live="polite">
              <TaskProgress task={task} />
              {task.status === "failed" ? <ManualDownloadLinks error={task.error} /> : null}
            </div>
          ) : null}
          {startError ? (
            <div className="flex flex-col gap-2">
              <ErrorMessage error={startError} />
              <ManualDownloadLinks error={startError} />
            </div>
          ) : null}

          <p className="text-sm text-muted">{t("modpacks.newInstanceHint")}</p>
          {filtered ? (
            <Switch
              checked={all}
              onChange={setAll}
              label={t("content.versions.showAll")}
              description={t("modpacks.versionsHint")}
            />
          ) : null}
          {versions.isPending ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : versions.error ? (
            <ErrorMessage error={versions.error} onRetry={() => void versions.refetch()} />
          ) : versions.data.length === 0 ? (
            <EmptyState title={t("content.versions.none")} />
          ) : (
            <ul
              aria-label={t("content.versions.title")}
              className="flex max-h-80 flex-col divide-y divide-border overflow-y-auto rounded-lg border border-border"
            >
              {versions.data.map((v) => (
                <li key={v.versionId} className="flex items-center gap-3 px-3 py-2 text-sm">
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium">{v.name}</p>
                    <p className="flex flex-wrap items-center gap-1.5 text-xs text-muted">
                      <span>{v.mcVersions.join(", ")}</span>
                      {v.loaders.map((l) => (
                        <Badge key={l}>{t(`loader.${l}`)}</Badge>
                      ))}
                      <span>· {new Date(v.published).toLocaleDateString(i18n.language)}</span>
                      <span>· {formatBytes(v.size)}</span>
                      {v.manualDownload ? <Badge tone="warning">{t("content.manualBadge")}</Badge> : null}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    icon={<Icon name="download" />}
                    disabled={busy}
                    aria-label={t("modpacks.installVersion", { version: v.versionNumber })}
                    onClick={() => install(v.versionId)}
                  >
                    {t("content.install")}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </Modal>
  );
}
