import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import type { Account, Instance, InstanceId, RunState } from "../../bindings";
import { api, unwrap } from "../../api/client";
import { qk, useAccounts, useInstances, useKillInstance, useStartTask } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Field, Select } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { isActive, useGameStore } from "../../stores/gameStore";
import { useNavStore } from "../../stores/navStore";
import { useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

const LOG_TAIL = 200;

const STATE_TONE: Record<RunState, "neutral" | "info" | "accent" | "danger"> = {
  stopped: "neutral",
  preparing: "info",
  running: "accent",
  crashed: "danger",
};

/** Most recently played first, never-played last. */
export function pickInstance(list: Instance[], preferred: InstanceId | null): Instance | undefined {
  const exact = preferred ? list.find((i) => i.id === preferred) : undefined;
  if (exact) return exact;
  return [...list].sort((a, b) => (b.lastPlayed ?? "").localeCompare(a.lastPlayed ?? ""))[0];
}

export function pickAccount(list: Account[], preferred: string | null): Account | undefined {
  return (preferred ? list.find((a) => a.id === preferred) : undefined) ?? list.find((a) => a.isDefault) ?? list[0];
}

function useRunState(id: InstanceId | undefined): RunState {
  const run = useGameStore((s) => (id ? s.runs[id] : undefined));
  const setState = useGameStore((s) => s.setState);
  const q = useQuery({
    queryKey: qk.instanceState(id ?? ""),
    queryFn: async () => unwrap(await api.instanceState(id ?? "")),
    enabled: !!id && !run,
  });
  useEffect(() => {
    if (id && q.data) setState(id, q.data);
  }, [id, q.data, setState]);
  return run?.state ?? q.data ?? "stopped";
}

function GameLog({ id }: { id: InstanceId }) {
  const { t } = useTranslation();
  const lines = useGameStore((s) => s.logs[id]);
  const clearLog = useGameStore((s) => s.clearLog);
  const tail = useMemo(() => (lines ?? []).slice(-LOG_TAIL), [lines]);
  const boxRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const el = boxRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [tail]);

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold">{t("play.log")}</h2>
        <Button size="sm" variant="ghost" disabled={tail.length === 0} onClick={() => clearLog(id)}>
          {t("play.clearLog")}
        </Button>
      </div>
      <pre
        ref={boxRef}
        aria-live="off"
        className="selectable h-64 overflow-auto rounded-xl border border-border bg-surface p-3 font-mono text-xs leading-relaxed text-muted"
      >
        {tail.length === 0 ? t("play.logEmpty") : tail.join("\n")}
      </pre>
    </section>
  );
}

export function PlayScreen() {
  const { t } = useTranslation();
  const instances = useInstances();
  const accounts = useAccounts();
  const nav = useNavStore();
  const setLaunchTask = useGameStore((s) => s.setLaunchTask);
  const clearLog = useGameStore((s) => s.clearLog);

  const instance = pickInstance(instances.data ?? [], nav.playInstance);
  const account = pickAccount(accounts.data ?? [], nav.playAccount);
  const state = useRunState(instance?.id);
  const run = useGameStore((s) => (instance ? s.runs[instance.id] : undefined));
  const launchTaskId = useGameStore((s) => (instance ? s.launchTasks[instance.id] : undefined));
  const launchTask = useTaskStore((s) => (launchTaskId ? s.tasks[launchTaskId] : undefined));

  const launch = useStartTask<{ id: InstanceId; account: string }>({
    run: (v, ch) =>
      api.instanceLaunch(v.id, { account: v.account, joinServer: null, room: null, offlineFast: false }, ch),
    invalidate: [qk.instances],
  });
  const kill = useKillInstance();

  if (instances.isPending || accounts.isPending) {
    return (
      <div role="status" aria-busy="true" aria-label={t("common.loading")} className="flex flex-col gap-4" data-testid="skeleton">
        <Skeleton className="h-40" />
        <Skeleton className="h-64" />
      </div>
    );
  }
  if (instances.error) return <ErrorMessage error={instances.error} onRetry={() => void instances.refetch()} />;

  if (!instance) {
    return (
      <EmptyState icon="layers" title={t("play.noInstances")}>
        <Button variant="primary" icon={<Icon name="plus" />} onClick={() => nav.go("instances")}>
          {t("instances.create")}
        </Button>
      </EmptyState>
    );
  }

  const launching = launchTask?.status === "running";
  const active = isActive(state);
  const canPlay = !!account && !active && !launching && !launch.isPending;

  const onPlay = () => {
    if (!account) return;
    clearLog(instance.id);
    launch.mutate(
      { id: instance.id, account: account.id },
      { onSuccess: (taskId) => setLaunchTask(instance.id, taskId) },
    );
  };

  return (
    <div className="flex flex-col gap-6">
      <section className="flex flex-col gap-5 rounded-2xl border border-border bg-surface p-6">
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t("play.instance")}>
            {(id) => (
              <Select id={id} value={instance.id} onChange={(e) => nav.selectPlayInstance(e.target.value)}>
                {(instances.data ?? []).map((i) => (
                  <option key={i.id} value={i.id}>
                    {i.name} · {i.mc}
                  </option>
                ))}
              </Select>
            )}
          </Field>
          <Field
            label={t("play.account")}
            error={accounts.data?.length === 0 ? "play.noAccounts" : null}
          >
            {(id, describedBy) => (
              <Select
                id={id}
                aria-describedby={describedBy}
                value={account?.id ?? ""}
                disabled={!accounts.data?.length}
                onChange={(e) => nav.selectPlayAccount(e.target.value)}
              >
                {(accounts.data ?? []).map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name} ({t(`accounts.kind.${a.kind}`)})
                  </option>
                ))}
              </Select>
            )}
          </Field>
        </div>

        <div className="flex flex-wrap items-center gap-4">
          {active ? (
            <Button
              size="lg"
              variant="danger"
              icon={<Icon name="stop" />}
              loading={kill.isPending}
              onClick={() => kill.mutate(instance.id)}
            >
              {t("play.stop")}
            </Button>
          ) : (
            <Button
              size="lg"
              variant="primary"
              icon={<Icon name="play" />}
              className="min-w-40"
              disabled={!canPlay}
              loading={launch.isPending || launching}
              onClick={onPlay}
            >
              {t("play.play")}
            </Button>
          )}
          <div className="flex flex-col gap-1 text-sm">
            <span className="flex items-center gap-2">
              {t("play.state")}: <Badge tone={STATE_TONE[state]}>{t(`runState.${state}`)}</Badge>
            </span>
            {state === "stopped" && run?.exitCode != null ? (
              <span className="text-xs text-muted">{t("play.exitCode", { code: run.exitCode })}</span>
            ) : null}
            {run?.crashReport ? (
              <span className="selectable text-xs text-danger">
                {t("play.crashReport", { path: run.crashReport })}
              </span>
            ) : null}
          </div>
        </div>

        {launchTask && (launchTask.status === "running" || launchTask.status === "failed") ? (
          <TaskProgress task={launchTask} compact />
        ) : null}
        <ErrorMessage error={launch.error} />
        <ErrorMessage error={kill.error} />
      </section>

      <GameLog id={instance.id} />
    </div>
  );
}
