import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ContentVersion, Instance, ProjectSummary } from "../../bindings";
import { useContentVersions } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { Switch } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { formatCount } from "../../lib/content";
import { useFormatBytes } from "../common/ExternalLink";
import { ProjectIcon } from "./ProjectIcon";

/** Project card with its versions; any version can be installed. */
export function ProjectDialog({
  instance,
  project,
  onClose,
  onInstall,
}: {
  instance: Instance;
  project: ProjectSummary | null;
  onClose: () => void;
  onInstall: (project: ProjectSummary, version: ContentVersion) => void;
}) {
  const { t, i18n } = useTranslation();
  const formatBytes = useFormatBytes();
  const [all, setAll] = useState(false);
  const loader = project?.kind === "mod" && instance.loader.kind !== "vanilla" ? instance.loader.kind : null;
  const filter = all ? { mc: null, loader: null } : { mc: instance.mc, loader };
  const versions = useContentVersions(project?.source ?? "modrinth", project?.projectId ?? null, filter);

  return (
    <Modal
      open={project !== null}
      onClose={onClose}
      wide
      title={project?.title ?? ""}
      footer={
        <Button variant="ghost" onClick={onClose}>
          {t("common.close")}
        </Button>
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
          <Switch
            checked={all}
            onChange={setAll}
            label={t("content.versions.showAll")}
            description={t("content.versions.compatibleHint", { mc: instance.mc, loader: t(`loader.${instance.loader.kind}`) })}
          />
          {versions.isPending ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-12" />
              <Skeleton className="h-12" />
            </div>
          ) : versions.error ? (
            <ErrorMessage error={versions.error} />
          ) : versions.data.length === 0 ? (
            <EmptyState title={t("content.versions.none")} />
          ) : (
            <ul className="flex flex-col divide-y divide-border rounded-lg border border-border">
              {versions.data.map((v) => (
                <li key={v.versionId} className="flex items-center gap-3 px-3 py-2 text-sm">
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-medium">{v.versionNumber}</p>
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
                  <Button size="sm" icon={<Icon name="download" />} onClick={() => onInstall(project, v)}>
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
