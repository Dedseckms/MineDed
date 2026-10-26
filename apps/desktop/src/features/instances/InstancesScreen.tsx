import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";

import type { Instance } from "../../bindings";
import { useDeleteInstance, useGameDir, useInstances } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Card, CardList } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { isActive, useGameStore } from "../../stores/gameStore";
import { Tabs } from "../../components/ui/Tabs";
import { LIBRARY_TABS, type LibraryTab, useNavStore } from "../../stores/navStore";
import { selectVisibleTasks, useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";
import { CloneInstanceDialog } from "./CloneInstanceDialog";
import { InstancePage } from "../content/InstancePage";
import { CreateInstanceDialog } from "./CreateInstanceDialog";
import { ImportDialog } from "./ImportDialog";
import { InstanceSettingsDialog } from "./InstanceSettingsDialog";
import { ModpackCatalog } from "./ModpackCatalog";

function GameDirRow({ id }: { id: string }) {
  const { t } = useTranslation();
  const dir = useGameDir(id);
  const [copied, setCopied] = useState(false);
  if (dir.error) return <ErrorMessage error={dir.error} className="mt-3" />;
  return (
    <div className="mt-3 flex items-center gap-2 rounded-lg bg-surface-2 px-3 py-2 text-xs">
      <Icon name="folder" className="size-4 shrink-0 text-muted" />
      <code className="selectable min-w-0 flex-1 truncate font-mono" title={dir.data}>
        {dir.data ?? t("common.loading")}
      </code>
      <Button
        size="sm"
        variant="ghost"
        disabled={!dir.data}
        onClick={() => {
          if (!dir.data) return;
          void navigator.clipboard
            ?.writeText(dir.data)
            .then(() => setCopied(true))
            .catch(() => setCopied(false));
        }}
      >
        {copied ? t("common.copied") : t("common.copy")}
      </Button>
    </div>
  );
}

function InstanceCard({
  instance,
  onSettings,
  onClone,
  onDelete,
}: {
  instance: Instance;
  onSettings: () => void;
  onClone: () => void;
  onDelete: () => void;
}) {
  const { t, i18n } = useTranslation();
  const go = useNavStore((s) => s.go);
  const select = useNavStore((s) => s.selectPlayInstance);
  const showInstance = useNavStore((s) => s.showInstance);
  const state = useGameStore((s) => s.runs[instance.id]?.state);
  const [showDir, setShowDir] = useState(false);
  const hours = Math.floor(instance.playtimeSec / 3600);

  return (
    <Card className="flex min-w-0 flex-col">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="truncate font-semibold">
            <button type="button" className="cursor-pointer truncate hover:underline" onClick={() => showInstance(instance.id)}>
              {instance.name}
            </button>
          </h3>
          <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
            <Badge>{instance.mc}</Badge>
            <Badge>{t(`loader.${instance.loader.kind}`)}</Badge>
            {isActive(state) ? <Badge tone="accent">{t(`runState.${state ?? "stopped"}`)}</Badge> : null}
            <span>
              {instance.lastPlayed
                ? t("instances.lastPlayed", {
                    date: new Date(instance.lastPlayed).toLocaleDateString(i18n.language),
                  })
                : t("instances.neverPlayed")}
            </span>
            {hours > 0 ? <span>· {t("instances.playtime", { count: hours })}</span> : null}
          </p>
        </div>
        <Button
          variant="primary"
          size="sm"
          icon={<Icon name="play" />}
          onClick={() => {
            select(instance.id);
            go("play");
          }}
        >
          {t("play.play")}
        </Button>
      </div>
      <div className="mt-3 flex flex-wrap gap-1">
        <Button size="sm" variant="ghost" icon={<Icon name="box" />} onClick={() => showInstance(instance.id)}>
          {t("instances.content")}
        </Button>
        <Button size="sm" variant="ghost" icon={<Icon name="sliders" />} onClick={onSettings}>
          {t("instances.settings")}
        </Button>
        <Button size="sm" variant="ghost" icon={<Icon name="copy" />} onClick={onClone}>
          {t("instances.clone")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          icon={<Icon name="folder" />}
          aria-expanded={showDir}
          onClick={() => setShowDir((v) => !v)}
        >
          {t("instances.folder")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto text-danger"
          icon={<Icon name="trash" />}
          disabled={isActive(state)}
          onClick={onDelete}
        >
          {t("common.delete")}
        </Button>
      </div>
      {showDir ? <GameDirRow id={instance.id} /> : null}
    </Card>
  );
}

export function InstancesScreen() {
  const openInstance = useNavStore((s) => s.openInstance);
  return openInstance ? <InstancePage id={openInstance} /> : <InstanceLibrary />;
}

const LIBRARY_TASKS = new Set(["instanceCreate", "instanceImport"]);

function InstanceLibrary() {
  const { t } = useTranslation();
  const instances = useInstances();
  const del = useDeleteInstance();
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  const [editing, setEditing] = useState<Instance | null>(null);
  const [cloning, setCloning] = useState<Instance | null>(null);
  const [deleting, setDeleting] = useState<Instance | null>(null);
  const tab = useNavStore((s) => s.libraryTab);
  const setTab = useNavStore((s) => s.setLibraryTab);
  const pending = useTaskStore(
    useShallow((s) =>
      selectVisibleTasks(s).filter((x) => x.kind !== null && LIBRARY_TASKS.has(x.kind) && x.status !== "succeeded"),
    ),
  );

  // Keep the dialog bound to fresh data after refetches.
  const editingFresh = editing ? (instances.data?.find((i) => i.id === editing.id) ?? editing) : null;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-muted">{t("instances.count", { count: instances.data?.length ?? 0 })}</p>
        <div className="flex flex-wrap gap-2">
          <Button icon={<Icon name="download" />} onClick={() => setImporting(true)}>
            {t("import.button")}
          </Button>
          <Button variant="primary" icon={<Icon name="plus" />} onClick={() => setCreating(true)}>
            {t("instances.create")}
          </Button>
        </div>
      </div>

      <Tabs<LibraryTab>
        value={tab}
        onChange={setTab}
        ariaLabel={t("instances.tabs.label")}
        idPrefix="library-tab"
        tabs={LIBRARY_TABS.map((v) => ({ value: v, label: t(`instances.tabs.${v}`) }))}
      />

      {pending.map((task) => (
        <div key={task.id} className="rounded-xl border border-border bg-surface p-4">
          <TaskProgress task={task} />
        </div>
      ))}

      {tab === "catalog" ? (
        <div role="tabpanel" aria-labelledby="library-tab-catalog">
          <ModpackCatalog />
        </div>
      ) : instances.isPending ? (
        <ListSkeleton rows={4} rowClassName="h-28" className="grid gap-3 lg:grid-cols-2" />
      ) : instances.error ? (
        <ErrorMessage error={instances.error} onRetry={() => void instances.refetch()} />
      ) : instances.data.length === 0 ? (
        <EmptyState icon="layers" title={t("instances.empty")}>
          <Button variant="primary" icon={<Icon name="plus" />} onClick={() => setCreating(true)}>
            {t("instances.create")}
          </Button>
        </EmptyState>
      ) : (
        <CardList className="grid gap-3 lg:grid-cols-2">
          {instances.data.map((inst) => (
            <InstanceCard
              key={inst.id}
              instance={inst}
              onSettings={() => setEditing(inst)}
              onClone={() => setCloning(inst)}
              onDelete={() => {
                del.reset();
                setDeleting(inst);
              }}
            />
          ))}
        </CardList>
      )}

      <CreateInstanceDialog open={creating} onClose={() => setCreating(false)} />
      <ImportDialog open={importing} onClose={() => setImporting(false)} />
      <InstanceSettingsDialog instance={editingFresh} onClose={() => setEditing(null)} />
      <CloneInstanceDialog source={cloning} onClose={() => setCloning(null)} />
      <ConfirmDialog
        open={deleting !== null}
        danger
        title={t("instances.deleteTitle")}
        message={t("instances.deleteMessage", { name: deleting?.name ?? "" })}
        confirmLabel={t("common.delete")}
        loading={del.isPending}
        error={del.error ? <ErrorMessage error={del.error} /> : null}
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          if (deleting) del.mutate(deleting.id, { onSuccess: () => setDeleting(null) });
        }}
      />
    </div>
  );
}
