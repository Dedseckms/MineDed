import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";

import type { Instance, ProjectSummary, SortBy } from "../../bindings";
import { useContentSearch, useInstallModpack, useVersions } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { VirtualList } from "../../components/ui/VirtualList";
import { Segmented, Select, TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { useDebounced } from "../../hooks/useDebounced";
import { SEARCH_SOURCES, SORTS, type SearchSource, formatCount } from "../../lib/content";
import { MODPACK_LOADERS, type ModpackLoader, buildModpackQuery } from "../../lib/modpacks";
import { selectModpackFilters, useModpackStore } from "../../stores/modpackStore";
import { useNavStore } from "../../stores/navStore";
import { useTaskView } from "../../stores/taskStore";
import { ProjectIcon } from "../content/ProjectIcon";
import { ModpackDialog } from "./ModpackDialog";

const RELEASES = ["release"] as const;
const ANY = "";

function ModpackCard({
  project,
  installing,
  onOpen,
  onInstall,
}: {
  project: ProjectSummary;
  installing: boolean;
  onOpen: () => void;
  onInstall: () => void;
}) {
  const { t, i18n } = useTranslation();
  return (
    <div className="flex h-full gap-3 overflow-hidden rounded-xl border border-border bg-surface p-4">
      <ProjectIcon project={project} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <button type="button" onClick={onOpen} className="cursor-pointer truncate font-semibold hover:underline">
            {project.title}
          </button>
          <span className="text-xs text-muted">{t("content.by", { author: project.author })}</span>
        </div>
        <p className="mt-0.5 line-clamp-2 text-sm text-muted">{project.description}</p>
        <p className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted">
          <span>{t("content.downloads", { count: formatCount(project.downloads, i18n.language) })}</span>
          {project.categories.slice(0, 3).map((c) => (
            <Badge key={c}>{c}</Badge>
          ))}
        </p>
      </div>
      <div className="flex shrink-0 flex-col gap-1">
        <Button size="sm" variant="primary" icon={<Icon name="download" />} loading={installing} onClick={onInstall}>
          {t("content.install")}
        </Button>
        <Button size="sm" variant="ghost" onClick={onOpen}>
          {t("content.versions.title")}
        </Button>
      </div>
    </div>
  );
}

const ROW = 148;
const VIEWPORT = 640;

/** Ready-made modpacks from Modrinth / CurseForge; each installs as a new instance. */
export function ModpackCatalog() {
  const { t } = useTranslation();
  const filters = useModpackStore(useShallow(selectModpackFilters));
  const store = useModpackStore(
    useShallow((s) => ({
      setSource: s.setSource,
      setText: s.setText,
      setMc: s.setMc,
      setLoader: s.setLoader,
      setSort: s.setSort,
    })),
  );
  const [text, setText] = useState(filters.text);
  const debounced = useDebounced(text, 300);
  const setStoreText = store.setText;
  useEffect(() => setStoreText(debounced), [debounced, setStoreText]);

  const versions = useVersions([...RELEASES]);
  const search = useContentSearch(buildModpackQuery(filters, 0));
  const items = search.data?.pages.flatMap((p) => p.items) ?? [];
  const total = search.data?.pages[0]?.total ?? 0;
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = search;
  const loadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const [open, setOpen] = useState<ProjectSummary | null>(null);
  const showInstance = useNavStore((s) => s.showInstance);
  const onInstalled = useCallback(
    (inst: Instance) => {
      setOpen(null);
      // Only jump if the user is still in the instances section.
      if (useNavStore.getState().section === "instances") showInstance(inst.id);
    },
    [showInstance],
  );
  const installer = useInstallModpack(onInstalled);
  const task = useTaskView(installer.data);
  const installingId = installer.isPending || task?.status === "running" ? installer.variables?.projectId : undefined;
  const ofOpen =
    open !== null && installer.variables?.projectId === open.projectId && installer.variables.source === open.source;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
        <div className="flex flex-wrap items-center gap-2">
          <Segmented<SearchSource>
            ariaLabel={t("content.filters.source")}
            value={filters.source}
            onChange={store.setSource}
            options={SEARCH_SOURCES.map((s) => ({ value: s, label: t(`content.source.${s}`) }))}
          />
          <Select
            aria-label={t("modpacks.filters.mc")}
            className="w-auto"
            value={filters.mc ?? ANY}
            onChange={(e) => store.setMc(e.target.value === ANY ? null : e.target.value)}
          >
            <option value={ANY}>{t("modpacks.filters.anyVersion")}</option>
            {(versions.data ?? []).map((v) => (
              <option key={v.id} value={v.id}>
                {v.id}
              </option>
            ))}
          </Select>
          <Select
            aria-label={t("modpacks.filters.loader")}
            className="w-auto"
            value={filters.loader ?? ANY}
            onChange={(e) => store.setLoader(e.target.value === ANY ? null : (e.target.value as ModpackLoader))}
          >
            <option value={ANY}>{t("modpacks.filters.anyLoader")}</option>
            {MODPACK_LOADERS.map((l) => (
              <option key={l} value={l}>
                {t(`loader.${l}`)}
              </option>
            ))}
          </Select>
          <Select
            aria-label={t("content.filters.sort")}
            className="w-auto"
            value={filters.sort}
            onChange={(e) => store.setSort(e.target.value as SortBy)}
          >
            {SORTS.map((s) => (
              <option key={s} value={s}>
                {t(`content.sort.${s}`)}
              </option>
            ))}
          </Select>
        </div>
        <div className="relative">
          <Icon name="search" className="pointer-events-none absolute top-3 left-3 size-4 text-muted" />
          <TextInput
            type="search"
            aria-label={t("content.filters.search")}
            className="pl-9"
            value={text}
            placeholder={t("modpacks.searchPlaceholder")}
            onChange={(e) => setText(e.target.value)}
          />
        </div>
      </div>

      {installer.error && !open ? <ErrorMessage error={installer.error} /> : null}

      {search.isPending ? (
        <ListSkeleton rows={4} rowClassName="h-24" className="flex flex-col gap-3" />
      ) : search.error ? (
        <ErrorMessage error={search.error} onRetry={() => void search.refetch()} />
      ) : items.length === 0 ? (
        <EmptyState icon="search" title={t("modpacks.noResults")} />
      ) : (
        <>
          <p className="text-xs text-muted" aria-live="polite">
            {t("content.results", { shown: items.length, total })}
          </p>
          <div className={search.isPlaceholderData ? "opacity-60" : ""}>
            <VirtualList
              items={items}
              rowHeight={ROW}
              height={VIEWPORT}
              ariaLabel={t("modpacks.listLabel")}
              getKey={(p) => `${p.source}:${p.projectId}`}
              onEndReached={loadMore}
              renderRow={(p) => (
                <div className="h-full pb-3">
                  <ModpackCard
                    project={p}
                    installing={installingId === p.projectId}
                    onOpen={() => setOpen(p)}
                    onInstall={() => {
                      // Progress is shown in the dialog and in the task center.
                      setOpen(p);
                      installer.install({ source: p.source, projectId: p.projectId, versionId: null });
                    }}
                  />
                </div>
              )}
            />
          </div>
          <div className="flex justify-center py-2">
            {hasNextPage ? (
              <Button variant="ghost" loading={isFetchingNextPage} onClick={() => void fetchNextPage()}>
                {t("content.loadMore")}
              </Button>
            ) : (
              <span className="text-xs text-muted">{t("content.end")}</span>
            )}
          </div>
        </>
      )}

      <ModpackDialog
        project={open}
        filters={filters}
        task={ofOpen ? task : undefined}
        starting={ofOpen && installer.isPending}
        startError={ofOpen ? installer.error : null}
        onInstall={installer.install}
        onClose={() => setOpen(null)}
      />
    </div>
  );
}
