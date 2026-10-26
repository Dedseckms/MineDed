import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ContentKind, InstalledContent, Instance, ProjectSummary } from "../../bindings";
import { api } from "../../api/client";
import { qk, useContentList, useInstances, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Tabs } from "../../components/ui/Tabs";
import { Badge, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { type SearchKind, hasShaderLoader, installedOfKind, shaderLoaderFor } from "../../lib/content";
import { useContentStore } from "../../stores/contentStore";
import { INSTANCE_TABS, type InstanceTab, useNavStore } from "../../stores/navStore";
import { useTaskView } from "../../stores/taskStore";
import { ExternalLink } from "../common/ExternalLink";
import { ExportDialog } from "../instances/ExportDialog";
import { PresetDialog } from "../instances/PresetDialog";
import { TaskProgress } from "../tasks/TaskProgress";
import { Catalog } from "./Catalog";
import { InstallPlanDialog, type InstallRequest } from "./InstallPlanDialog";
import { InstalledList } from "./InstalledList";
import { ProjectDialog } from "./ProjectDialog";

const TAB_KIND: Record<Exclude<InstanceTab, "browse">, ContentKind> = {
  mods: "mod",
  resourcePacks: "resourcePack",
  shaders: "shader",
};

export const OPTIFINE_URL = "https://optifine.net/downloads";
/** Modrinth project ids of the shader loaders. */
const SHADER_LOADER_IDS = { iris: "YL57xq9U", oculus: "GchcoXML" } as const;

function ShaderHint({ instance, installed, onInstall }: { instance: Instance; installed: InstalledContent[]; onInstall: (r: InstallRequest) => void }) {
  const { t } = useTranslation();
  const loader = shaderLoaderFor(instance.loader.kind);
  const present = hasShaderLoader(installed);
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-4 text-sm">
      <p className="font-medium">{t("content.shaders.hintTitle")}</p>
      {loader === null ? (
        <p className="text-warning">{t("content.shaders.vanillaWarning")}</p>
      ) : present ? (
        <p className="text-muted">{t("content.shaders.ready", { loader: t(`content.shaders.loaderName.${loader}`) })}</p>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <p className="text-warning">{t("content.shaders.missingLoader", { loader: t(`content.shaders.loaderName.${loader}`) })}</p>
          <Button
            size="sm"
            variant="primary"
            icon={<Icon name="download" />}
            onClick={() =>
              onInstall({
                picks: [{ source: "modrinth", projectId: SHADER_LOADER_IDS[loader], versionId: null }],
                titles: { [SHADER_LOADER_IDS[loader]]: t(`content.shaders.loaderName.${loader}`) },
                shaders: false,
              })
            }
          >
            {t("content.shaders.installLoader", { loader: t(`content.shaders.loaderName.${loader}`) })}
          </Button>
        </div>
      )}
      <p className="text-xs text-muted">
        {t("content.shaders.optifine")} <ExternalLink href={OPTIFINE_URL}>{t("content.shaders.optifineLink")}</ExternalLink>
      </p>
    </div>
  );
}

function UpdateAll({ instance, items }: { instance: Instance; items: InstalledContent[] }) {
  const { t } = useTranslation();
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskView(taskId);
  const update = useStartTask<null>({
    run: (_v, ch) => api.contentUpdateAll(instance.id, ch),
    invalidate: [qk.content(instance.id)],
  });
  const count = items.filter((c) => c.updateAvailable).length;
  if (count === 0 && !task) return null;
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3">
      <div className="flex items-center gap-3">
        <Icon name="refresh" className="size-4 text-info" />
        <span className="flex-1 text-sm">{t("content.updates", { count })}</span>
        <Button
          size="sm"
          variant="primary"
          disabled={count === 0 || task?.status === "running"}
          loading={update.isPending}
          onClick={() => update.mutate(null, { onSuccess: setTaskId })}
        >
          {t("content.updateAll")}
        </Button>
      </div>
      {task ? <TaskProgress task={task} compact /> : null}
      <ErrorMessage error={update.error} />
    </div>
  );
}

function InstanceContent({ instance }: { instance: Instance }) {
  const { t } = useTranslation();
  const tab = useNavStore((s) => s.instanceTab);
  const setTab = useNavStore((s) => s.setInstanceTab);
  const browseKind = useContentStore((s) => s.browse);
  const content = useContentList(instance.id);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [request, setRequest] = useState<InstallRequest | null>(null);
  const installed = content.data ?? [];

  const browse = (kind: SearchKind) => {
    browseKind(kind);
    setTab("browse");
  };

  const installProject = (p: ProjectSummary, versionId: string | null = null) =>
    setRequest({
      picks: [{ source: p.source, projectId: p.projectId, versionId }],
      titles: { [p.projectId]: p.title },
      shaders: p.kind === "shader",
    });

  const count = (kind: ContentKind) => installed.filter((c) => c.kind === kind).length;

  return (
    <div className="flex flex-col gap-4">
      <Tabs<InstanceTab>
        idPrefix="inst-tab"
        ariaLabel={t("content.tabs.label")}
        value={tab}
        onChange={setTab}
        tabs={INSTANCE_TABS.map((v) => ({
          value: v,
          label: t(`content.tabs.${v}`),
          badge: v !== "browse" && content.data ? <Badge>{count(TAB_KIND[v])}</Badge> : undefined,
        }))}
      />
      <div role="tabpanel" aria-labelledby={`inst-tab-${tab}`} className="flex flex-col gap-4">
        {tab === "browse" ? (
          <Catalog instance={instance} installed={installed} onOpen={setProject} onInstall={(p) => installProject(p)} />
        ) : content.isPending ? (
          <ListSkeleton rows={5} rowClassName="h-14" />
        ) : content.error ? (
          <ErrorMessage error={content.error} onRetry={() => void content.refetch()} />
        ) : (
          <>
            {tab === "shaders" ? <ShaderHint instance={instance} installed={installed} onInstall={setRequest} /> : null}
            {tab === "mods" ? <UpdateAll instance={instance} items={installed} /> : null}
            <InstalledList
              key={tab}
              instance={instance}
              kind={TAB_KIND[tab]}
              items={installedOfKind(installed, TAB_KIND[tab])}
              onBrowse={() => browse(TAB_KIND[tab] as SearchKind)}
              onInstall={setRequest}
            />
          </>
        )}
      </div>

      <ProjectDialog
        instance={instance}
        project={project}
        onClose={() => setProject(null)}
        onInstall={(p, v) => {
          setProject(null);
          installProject(p, v.versionId);
        }}
      />
      <InstallPlanDialog instance={instance} installed={installed} request={request} onClose={() => setRequest(null)} />
    </div>
  );
}

/** Instance page: header with actions and tabs for mods, resource packs, shaders and the catalog. */
export function InstancePage({ id }: { id: string }) {
  const { t } = useTranslation();
  const instances = useInstances();
  const show = useNavStore((s) => s.showInstance);
  const go = useNavStore((s) => s.go);
  const selectPlay = useNavStore((s) => s.selectPlayInstance);
  const [presetOpen, setPresetOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const instance = instances.data?.find((i) => i.id === id);

  if (instances.isPending) return <ListSkeleton rows={3} rowClassName="h-20" />;
  if (instances.error) return <ErrorMessage error={instances.error} onRetry={() => void instances.refetch()} />;

  return (
    <div className="flex flex-col gap-4">
      <div>
        <Button size="sm" variant="ghost" icon={<Icon name="back" />} onClick={() => show(null)}>
          {t("content.back")}
        </Button>
      </div>
      {!instance ? (
        <ErrorMessage error={{ code: "notFound", message: "instance", details: null }} />
      ) : (
        <>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <h2 className="truncate text-xl font-semibold">{instance.name}</h2>
              <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                <Badge>{instance.mc}</Badge>
                <Badge>
                  {t(`loader.${instance.loader.kind}`)}
                  {instance.loader.version ? ` ${instance.loader.version}` : ""}
                </Badge>
                {instance.preset ? <Badge tone="accent">{t(`preset.${instance.preset}.title`)}</Badge> : null}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button icon={<Icon name="zap" />} onClick={() => setPresetOpen(true)}>
                {t("preset.button")}
              </Button>
              <Button icon={<Icon name="upload" />} onClick={() => setExportOpen(true)}>
                {t("instances.export")}
              </Button>
              <Button
                variant="primary"
                icon={<Icon name="play" />}
                onClick={() => {
                  selectPlay(instance.id);
                  go("play");
                }}
              >
                {t("play.play")}
              </Button>
            </div>
          </div>
          <InstanceContent instance={instance} />
          <PresetDialog instance={presetOpen ? instance : null} onClose={() => setPresetOpen(false)} />
          <ExportDialog instance={exportOpen ? instance : null} onClose={() => setExportOpen(false)} />
        </>
      )}
    </div>
  );
}
