import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { VpnMode, VpnNode, VpnState } from "../../bindings";
import { api } from "../../api/client";
import {
  qk,
  useImportVpnFile,
  useSelectVpnNode,
  useSetVpnMode,
  useSetVpnSubscription,
  useStartTask,
  useTestVpnLatency,
  useVpnNodes,
  useVpnState,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { VirtualList } from "../../components/ui/VirtualList";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { Field, Segmented, TextInput } from "../../components/ui/form";
import {
  type NodeSort,
  VPN_MODES,
  filterNodes,
  latencyTone,
  parseSubscriptionUrls,
  sortNodes,
} from "../../lib/vpn";
import { pickFile } from "../../api/dialog";
import { useTaskView } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";
import { AppsSection } from "./AppsSection";
import { MinededCard } from "./MinededCard";

const NODE_ROW = 52;
const NODE_VIEWPORT = 416;
/** Show the search box only when the list is long enough to need it. */
const SEARCH_FROM = 8;

function Latency({ ms }: { ms: number | null }) {
  const { t } = useTranslation();
  return <Badge tone={latencyTone(ms)}>{ms === null ? t("vpn.latency.unknown") : t("vpn.latency.ms", { ms })}</Badge>;
}

function StatusSection({ state, hasNodes }: { state: VpnState; hasNodes: boolean }) {
  const { t, i18n } = useTranslation();
  const setMode = useSetVpnMode();
  const pending = setMode.isPending ? setMode.variables : null;
  const mode = pending ?? state.mode;

  return (
    <Section
      title={t("vpn.status.title")}
      actions={
        <Badge tone={state.error ? "danger" : state.running ? "accent" : "neutral"}>
          {state.error ? t("vpn.status.error") : state.running ? t("vpn.status.running") : t("vpn.status.stopped")}
        </Badge>
      }
    >
      <div className="flex flex-col gap-2">
        <Segmented<VpnMode>
          ariaLabel={t("vpn.mode.label")}
          value={mode}
          onChange={(m) => {
            if (m !== mode) setMode.mutate(m);
          }}
          options={VPN_MODES.map((m) => ({
            value: m,
            label: t(`vpn.mode.${m}`),
            disabled: setMode.isPending || (m !== "off" && !hasNodes),
          }))}
        />
        <p className="text-xs text-muted">{t(`vpn.modeHint.${mode}`)}</p>
        {mode === "apps" || setMode.variables === "apps" ? <p className="text-xs text-warning">{t("vpn.mode.adminNote")}</p> : null}
        {!hasNodes ? <p className="text-xs text-warning">{t("vpn.mode.needNodes")}</p> : null}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
        <dt className="text-muted">{t("vpn.status.node")}</dt>
        <dd className="flex min-w-0 items-center gap-2">
          <span className="truncate">{state.node ?? t("vpn.status.noNode")}</span>
          {state.node ? <Latency ms={state.latencyMs} /> : null}
        </dd>
        <dt className="text-muted">{t("vpn.status.updated")}</dt>
        <dd>
          {state.subscriptionUpdated
            ? new Date(state.subscriptionUpdated).toLocaleString(i18n.language)
            : t("vpn.status.never")}
        </dd>
      </dl>
      <ErrorMessage error={state.error} />
      <ErrorMessage error={setMode.error} />
    </Section>
  );
}

type SubSource = "url" | "file";

function SubscriptionSection() {
  const { t } = useTranslation();
  const [source, setSource] = useState<SubSource>("url");
  const [text, setText] = useState("");
  const [pickError, setPickError] = useState<unknown>(null);
  const [touched, setTouched] = useState(false);
  const setSub = useSetVpnSubscription();
  const importFile = useImportVpnFile();
  const refresh = useStartTask<void>({
    run: (_v, ch) => api.vpnRefresh(ch),
    invalidate: [qk.vpnState, qk.vpnNodes],
  });
  const task = useTaskView(refresh.data);
  const refreshing = refresh.isPending || task?.status === "running";

  const parsed = parseSubscriptionUrls(text);
  const urlError = parsed.ok ? null : parsed.error;

  return (
    <Section
      title={t("vpn.sub.title")}
      actions={
        <Button size="sm" icon={<Icon name="refresh" />} loading={refreshing} onClick={() => refresh.mutate()}>
          {t("vpn.sub.refresh")}
        </Button>
      }
    >
      <Segmented<SubSource>
        ariaLabel={t("vpn.sub.source")}
        value={source}
        onChange={(s) => {
          setSource(s);
          setTouched(false);
        }}
        options={[
          { value: "url", label: t("vpn.sub.byUrl") },
          { value: "file", label: t("vpn.sub.byFile") },
        ]}
      />
      {source === "url" ? (
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            setTouched(true);
            if (!parsed.ok) return;
            setSub.mutate(parsed.urls, {
              onSuccess: () => {
                setText("");
                setTouched(false);
                refresh.mutate();
              },
            });
          }}
        >
          <Field label={t("vpn.sub.urls")} error={touched ? urlError : null} hint={t("vpn.sub.urlsHint")}>
            {(id, d, inv) => (
              <textarea
                id={id}
                aria-describedby={d}
                aria-invalid={inv || undefined}
                rows={3}
                spellCheck={false}
                autoComplete="off"
                value={text}
                placeholder={t("vpn.sub.urlsPlaceholder")}
                onChange={(e) => {
                  setText(e.target.value);
                  setSub.reset();
                }}
                className={`w-full resize-y rounded-lg border bg-surface-2 px-3 py-2 font-mono text-sm text-text outline-none placeholder:text-muted focus:border-accent ${inv ? "border-danger" : "border-border"}`}
              />
            )}
          </Field>
          <div className="flex justify-end">
            <Button type="submit" variant="primary" loading={setSub.isPending}>
              {t("vpn.sub.save")}
            </Button>
          </div>
          <ErrorMessage error={setSub.error} />
        </form>
      ) : (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="primary"
              loading={importFile.isPending}
              icon={<Icon name="upload" />}
              onClick={() => {
                importFile.reset();
                setPickError(null);
                pickFile()
                  .then((path) => {
                    if (path) importFile.mutate(path);
                  })
                  .catch(setPickError);
              }}
            >
              {t("vpn.sub.pickFile")}
            </Button>
            <p className="min-w-0 flex-1 text-xs text-muted">{t("vpn.sub.fileHint")}</p>
          </div>
          <ErrorMessage error={importFile.error ?? pickError} />
        </div>
      )}
      {task ? <TaskProgress task={task} compact /> : null}
      <ErrorMessage error={refresh.error} />
    </Section>
  );
}

function NodeRow({ node, onSelect, busy }: { node: VpnNode; onSelect: () => void; busy: boolean }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      role="radio"
      aria-checked={node.selected}
      disabled={busy}
      onClick={onSelect}
      title={t("vpn.nodes.select")}
      className={`mx-1 flex h-[46px] w-[calc(100%-0.5rem)] cursor-pointer items-center gap-3 rounded-lg border px-3 text-left text-sm transition-colors disabled:cursor-wait ${
        node.selected ? "border-accent bg-accent/10" : "border-transparent hover:bg-surface-2"
      }`}
    >
      <span className={`grid size-4 shrink-0 place-items-center rounded-full border ${node.selected ? "border-accent bg-accent text-accent-fg" : "border-border"}`}>
        {node.selected ? <Icon name="check" className="size-3" /> : null}
      </span>
      <span className="min-w-0 flex-1 truncate font-medium">{node.name}</span>
      <Badge>{node.protocol}</Badge>
      <Latency ms={node.latencyMs} />
    </button>
  );
}

function NodesSection({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const nodes = useVpnNodes();
  const select = useSelectVpnNode();
  const ping = useTestVpnLatency();
  const [sort, setSort] = useState<NodeSort>("latency");
  const [query, setQuery] = useState("");

  const visible = useMemo(() => sortNodes(filterNodes(nodes.data ?? [], query), sort), [nodes.data, query, sort]);
  const total = nodes.data?.length ?? 0;

  return (
    <Section
      title={t("vpn.nodes.title", { n: total })}
      actions={
        <Button
          size="sm"
          icon={<Icon name="zap" />}
          loading={ping.isPending}
          disabled={total === 0 || !running}
          title={running ? undefined : t("vpn.nodes.pingNeedsVpn")}
          aria-describedby={!running && total > 0 ? "vpn-ping-hint" : undefined}
          onClick={() => ping.mutate()}
        >
          {t("vpn.nodes.pingAll")}
        </Button>
      }
    >
      {!running && total > 0 ? (
        <p id="vpn-ping-hint" className="text-xs text-muted">
          {t("vpn.nodes.pingNeedsVpn")}
        </p>
      ) : null}
      {nodes.isPending ? (
        <ListSkeleton rows={5} rowClassName="h-[46px]" />
      ) : nodes.error ? (
        <ErrorMessage error={nodes.error} onRetry={() => void nodes.refetch()} />
      ) : total === 0 ? (
        <EmptyState icon="shield" title={t("vpn.nodes.empty")} description={t("vpn.nodes.emptyHint")} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            {total >= SEARCH_FROM ? (
              <TextInput
                type="search"
                className="max-w-xs"
                aria-label={t("vpn.nodes.search")}
                placeholder={t("vpn.nodes.search")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            ) : null}
            <Segmented<NodeSort>
              ariaLabel={t("vpn.nodes.sort")}
              value={sort}
              onChange={setSort}
              options={[
                { value: "latency", label: t("vpn.nodes.byLatency") },
                { value: "name", label: t("vpn.nodes.byName") },
              ]}
            />
          </div>
          {visible.length === 0 ? (
            <p className="text-sm text-muted">{t("vpn.nodes.noMatch")}</p>
          ) : (
            <div role="radiogroup" aria-label={t("vpn.nodes.label")}>
              <VirtualList
                items={visible}
                rowHeight={NODE_ROW}
                height={NODE_VIEWPORT}
                getKey={(n) => n.name}
                renderRow={(n) => (
                  <NodeRow node={n} busy={select.isPending} onSelect={() => !n.selected && select.mutate(n.name)} />
                )}
              />
            </div>
          )}
        </>
      )}
      <ErrorMessage error={select.error} />
      <ErrorMessage error={ping.error} />
    </Section>
  );
}

export function VpnScreen() {
  const { t } = useTranslation();
  const state = useVpnState();
  const nodes = useVpnNodes();
  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm text-muted">{t("vpn.intro")}</p>
      <MinededCard state={state.data} />
      {state.isPending ? (
        <ListSkeleton rows={1} rowClassName="h-40" />
      ) : state.error ? (
        <ErrorMessage error={state.error} onRetry={() => void state.refetch()} />
      ) : (
        <StatusSection state={state.data} hasNodes={(nodes.data?.length ?? 0) > 0} />
      )}
      <AppsSection />
      <SubscriptionSection />
      <NodesSection running={state.data?.running ?? false} />
    </div>
  );
}
