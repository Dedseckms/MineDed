import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";

import type { InstalledContent, Instance, ProjectSummary, SortBy } from "../../bindings";
import { useContentSearch } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { VirtualList } from "../../components/ui/VirtualList";
import { Segmented, Select, Switch, TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { useDebounced } from "../../hooks/useDebounced";
import {
  SEARCH_KINDS,
  SEARCH_SOURCES,
  SORTS,
  type SearchKind,
  type SearchSource,
  buildSearchQuery,
  formatCount,
} from "../../lib/content";
import { selectFilters, useContentStore } from "../../stores/contentStore";
import { ProjectIcon } from "./ProjectIcon";

function ResultCard({
  project,
  installed,
  onOpen,
  onInstall,
}: {
  project: ProjectSummary;
  installed: boolean;
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
          {installed ? <Badge tone="accent">{t("content.installed")}</Badge> : null}
        </p>
      </div>
      <div className="flex shrink-0 flex-col gap-1">
        <Button size="sm" variant="primary" icon={<Icon name="download" />} onClick={onInstall}>
          {t("content.install")}
        </Button>
        <Button size="sm" variant="ghost" onClick={onOpen}>
          {t("content.versions.title")}
        </Button>
      </div>
    </div>
  );
}

const RESULT_ROW = 148;
const RESULT_VIEWPORT = 640;

/** Modrinth / CurseForge search with instance-aware filters and infinite scroll. */
export function Catalog({
  instance,
  installed,
  onOpen,
  onInstall,
}: {
  instance: Instance;
  installed: InstalledContent[];
  onOpen: (p: ProjectSummary) => void;
  onInstall: (p: ProjectSummary) => void;
}) {
  const { t } = useTranslation();
  const filters = useContentStore(useShallow(selectFilters));
  const store = useContentStore(
    useShallow((s) => ({
      setSource: s.setSource,
      setKind: s.setKind,
      setText: s.setText,
      setSort: s.setSort,
      setCompatibleOnly: s.setCompatibleOnly,
    })),
  );
  const [text, setText] = useState(filters.text);
  const debounced = useDebounced(text, 300);
  const setStoreText = store.setText;
  useEffect(() => setStoreText(debounced), [debounced, setStoreText]);

  const query = buildSearchQuery(filters, instance, 0);
  const search = useContentSearch(query);
  const items = search.data?.pages.flatMap((p) => p.items) ?? [];
  const total = search.data?.pages[0]?.total ?? 0;
  const installedIds = new Set(installed.map((c) => c.projectId).filter((x): x is string => x !== null));

  // Infinite scroll: the windowed list asks for more when its last rows show up.
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = search;
  const loadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

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
            aria-label={t("content.filters.kind")}
            className="w-auto"
            value={filters.kind}
            onChange={(e) => store.setKind(e.target.value as SearchKind)}
          >
            {SEARCH_KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`content.kind.${k}`)}
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
            placeholder={t("content.filters.searchPlaceholder")}
            onChange={(e) => setText(e.target.value)}
          />
        </div>
        <Switch
          checked={filters.compatibleOnly}
          onChange={store.setCompatibleOnly}
          label={t("content.filters.compatible")}
          description={
            filters.kind === "mod" && instance.loader.kind !== "vanilla"
              ? t("content.filters.compatibleHintLoader", { mc: instance.mc, loader: t(`loader.${instance.loader.kind}`) })
              : t("content.filters.compatibleHint", { mc: instance.mc })
          }
        />
      </div>

      {filters.kind === "mod" && instance.loader.kind === "vanilla" && filters.compatibleOnly ? (
        <div role="status" className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
          {t("content.vanillaMods")}
        </div>
      ) : null}

      {search.isPending ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : search.error ? (
        <ErrorMessage error={search.error} />
      ) : items.length === 0 ? (
        <EmptyState title={t("content.noResults")} />
      ) : (
        <>
          <p className="text-xs text-muted" aria-live="polite">
            {t("content.results", { shown: items.length, total })}
          </p>
          <div className={search.isPlaceholderData ? "opacity-60" : ""}>
            <VirtualList
              items={items}
              rowHeight={RESULT_ROW}
              height={RESULT_VIEWPORT}
              ariaLabel={t("content.tabs.browse")}
              getKey={(p) => `${p.source}:${p.projectId}`}
              onEndReached={loadMore}
              renderRow={(p) => (
                <div className="h-full pb-3">
                  <ResultCard
                    project={p}
                    installed={installedIds.has(p.projectId)}
                    onOpen={() => onOpen(p)}
                    onInstall={() => onInstall(p)}
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
    </div>
  );
}
