import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { ContentVersionRef, InstalledContent, Instance, ResolvePlan } from "../../bindings";
import { api } from "../../api/client";
import { qk, useResolveContent, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { Badge, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { shaderLoaderFor, shaderWarning, summarizePlan } from "../../lib/content";
import { useTaskView } from "../../stores/taskStore";
import { ExternalLink, useFormatBytes } from "../common/ExternalLink";
import { TaskProgress } from "../tasks/TaskProgress";

export interface InstallRequest {
  picks: ContentVersionRef[];
  /** Titles known by the caller (search results), by project id. */
  titles: Record<string, string>;
  /** The request installs shader packs (triggers the Iris/Oculus check). */
  shaders: boolean;
}

function titleLookup(plan: ResolvePlan, installed: InstalledContent[], known: Record<string, string>) {
  return (projectId: string) =>
    known[projectId] ??
    plan.install.find((v) => v.projectId === projectId)?.name ??
    installed.find((c) => c.projectId === projectId)?.title ??
    projectId;
}

/** Resolve → confirm plan (deps, conflicts, manual files, shader loader) → install with progress. */
export function InstallPlanDialog({
  instance,
  installed,
  request,
  onClose,
}: {
  instance: Instance;
  installed: InstalledContent[];
  request: InstallRequest | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const formatBytes = useFormatBytes();
  const resolve = useResolveContent();
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskView(taskId);
  const install = useStartTask<ResolvePlan>({
    run: (plan, ch) => api.contentInstall(instance.id, plan, ch),
    invalidate: [qk.content(instance.id)],
  });

  const resolveMutate = resolve.mutate;
  const resolveReset = resolve.reset;
  const installReset = install.reset;
  useEffect(() => {
    if (!request) return;
    setTaskId(null);
    installReset();
    resolveReset();
    resolveMutate({ instance: instance.id, picks: request.picks });
  }, [request, instance.id, resolveMutate, resolveReset, installReset]);

  const plan = resolve.data;
  const summary = plan ? summarizePlan(plan) : null;
  const title = plan && request ? titleLookup(plan, installed, request.titles) : (id: string) => id;
  const warning = plan && request ? shaderWarning(plan, installed, instance.loader.kind, request.shaders) : "none";
  const shaderLoader = shaderLoaderFor(instance.loader.kind);
  const picked = new Set(request?.picks.map((p) => p.projectId) ?? []);
  const running = task?.status === "running" || install.isPending;
  const done = task?.status === "succeeded";

  return (
    <Modal
      open={request !== null}
      onClose={onClose}
      wide
      title={t("content.plan.title")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {done ? t("common.close") : t("common.cancel")}
          </Button>
          {!done ? (
            <Button
              variant="primary"
              icon={<Icon name="download" />}
              disabled={!plan || !summary?.canInstall || taskId !== null}
              loading={running}
              onClick={() => {
                if (plan) install.mutate(plan, { onSuccess: setTaskId });
              }}
            >
              {summary && summary.installCount > 0
                ? t("content.plan.install", { count: summary.installCount, size: formatBytes(summary.totalBytes) })
                : t("content.install")}
            </Button>
          ) : null}
        </>
      }
    >
      {resolve.isPending ? (
        <div className="flex flex-col gap-2" aria-busy>
          <p className="text-sm text-muted">{t("content.plan.resolving")}</p>
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : resolve.error ? (
        <ErrorMessage error={resolve.error} />
      ) : plan ? (
        <div className="flex flex-col gap-4 text-sm">
          {warning !== "none" ? (
            <div role="status" className="flex gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-warning">
              <Icon name="alert" className="mt-0.5 size-4 shrink-0" />
              <span>
                {warning === "vanilla"
                  ? t("content.shaders.vanillaWarning")
                  : t("content.shaders.missingLoader", { loader: t(`content.shaders.loaderName.${shaderLoader ?? "iris"}`) })}
              </span>
            </div>
          ) : null}

          {plan.conflicts.length > 0 ? (
            <section className="flex flex-col gap-2">
              <h3 className="font-medium text-danger">{t("content.plan.conflicts")}</h3>
              <ul className="flex flex-col gap-1">
                {plan.conflicts.map((c) => (
                  <li key={`${c.projectId}-${c.withProjectId}`} className="rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-danger">
                    {t("content.plan.conflict", { a: title(c.projectId), b: title(c.withProjectId) })}
                  </li>
                ))}
              </ul>
              <p className="text-xs text-muted">{t("content.plan.conflictsHint")}</p>
            </section>
          ) : null}

          {plan.install.length > 0 ? (
            <section className="flex flex-col gap-2">
              <h3 className="font-medium">{t("content.plan.toInstall")}</h3>
              <ul className="flex flex-col divide-y divide-border rounded-lg border border-border">
                {plan.install.map((v) => (
                  <li key={v.versionId} className="flex items-center gap-3 px-3 py-2">
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-medium">{title(v.projectId)}</p>
                      <p className="truncate text-xs text-muted">
                        {v.versionNumber} · {v.fileName}
                      </p>
                    </div>
                    {!picked.has(v.projectId) ? <Badge tone="info">{t("content.plan.dependency")}</Badge> : null}
                    <span className="shrink-0 text-xs text-muted tabular-nums">{formatBytes(v.size)}</span>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {plan.manual.length > 0 ? (
            <section className="flex flex-col gap-2">
              <h3 className="font-medium">{t("content.plan.manual")}</h3>
              <p className="text-xs text-muted">{t("content.plan.manualHint")}</p>
              <ul className="flex flex-col gap-1">
                {plan.manual.map((m) => (
                  <li key={m.fileName} className="flex flex-col gap-0.5 rounded-lg bg-surface-2 px-3 py-2">
                    <span className="font-medium">{title(m.projectId)}</span>
                    <span className="truncate text-xs text-muted">{m.fileName}</span>
                    <ExternalLink href={m.url}>{t("content.plan.openPage")}</ExternalLink>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {plan.alreadyInstalled.length > 0 ? (
            <p className="text-xs text-muted">
              {t("content.plan.already", { list: plan.alreadyInstalled.map(title).join(", ") })}
            </p>
          ) : null}

          {plan.install.length === 0 && plan.manual.length === 0 && plan.conflicts.length === 0 ? (
            <p className="text-muted">{t("content.plan.nothing")}</p>
          ) : null}

          {task ? (
            <div className="rounded-lg border border-border p-3">
              <TaskProgress task={task} />
            </div>
          ) : null}
          <ErrorMessage error={install.error} />
        </div>
      ) : null}
    </Modal>
  );
}
