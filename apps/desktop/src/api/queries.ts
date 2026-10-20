// TanStack Query hooks over `api`. All backend access from components goes here.
import { type QueryKey, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  AccountId,
  AppError,
  InstanceId,
  InstancePatch,
  Settings,
  TaskEvent,
  TaskId,
  VersionKind,
} from "../bindings";
import { useTaskStore } from "../stores/taskStore";
import { createTaskChannel } from "./channel";
import { api, unwrap } from "./client";

export const qk = {
  instances: ["instances"] as const,
  instanceState: (id: InstanceId) => ["instanceState", id] as const,
  gameDir: (id: InstanceId) => ["gameDir", id] as const,
  ramRecommend: (id: InstanceId) => ["ramRecommend", id] as const,
  versions: (kinds: VersionKind[]) => ["versions", [...kinds].sort().join(",")] as const,
  accounts: ["accounts"] as const,
  yggServers: ["yggServers"] as const,
  settings: ["settings"] as const,
  systemInfo: ["systemInfo"] as const,
  tasks: ["tasks"] as const,
  appVersion: ["appVersion"] as const,
};

// ---- queries ---------------------------------------------------------------

export const useInstances = () =>
  useQuery({ queryKey: qk.instances, queryFn: async () => unwrap(await api.instanceList()) });

export const useAccounts = () =>
  useQuery({ queryKey: qk.accounts, queryFn: async () => unwrap(await api.accountList()) });

export const useSettings = () =>
  useQuery({ queryKey: qk.settings, queryFn: async () => unwrap(await api.settingsGet()) });

export const useSystemInfo = () =>
  useQuery({
    queryKey: qk.systemInfo,
    queryFn: async () => unwrap(await api.systemInfo()),
    staleTime: Infinity,
  });

export const useVersions = (kinds: VersionKind[]) =>
  useQuery({
    queryKey: qk.versions(kinds),
    queryFn: async () => unwrap(await api.versionsList({ kinds, search: null })),
    staleTime: 5 * 60_000,
  });

export const useYggdrasilServers = () =>
  useQuery({
    queryKey: qk.yggServers,
    queryFn: async () => unwrap(await api.accountYggdrasilServers()),
    staleTime: Infinity,
  });

export const useGameDir = (id: InstanceId | null) =>
  useQuery({
    queryKey: qk.gameDir(id ?? ""),
    queryFn: async () => unwrap(await api.instanceGameDir(id ?? "")),
    enabled: id !== null,
  });

export const useRamRecommend = (id: InstanceId | null) =>
  useQuery({
    queryKey: qk.ramRecommend(id ?? ""),
    queryFn: async () => unwrap(await api.ramRecommend(id ?? "")),
    enabled: id !== null,
  });

export const useAppVersion = () =>
  useQuery({ queryKey: qk.appVersion, queryFn: () => api.appVersion(), staleTime: Infinity });

// ---- plain mutations -------------------------------------------------------

/** Saves the whole settings object; the cache is updated optimistically. */
export function useSaveSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (next: Settings) => unwrap(await api.settingsSet(next)),
    onMutate: async (next) => {
      await qc.cancelQueries({ queryKey: qk.settings });
      const prev = qc.getQueryData<Settings>(qk.settings);
      qc.setQueryData(qk.settings, next);
      return { prev };
    },
    onError: (_e, _next, ctx) => {
      if (ctx?.prev) qc.setQueryData(qk.settings, ctx.prev);
    },
    onSettled: () => qc.invalidateQueries({ queryKey: qk.settings }),
  });
}

export function useAddOfflineAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) => unwrap(await api.accountOfflineAdd(name)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

export function useYggdrasilLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { apiRoot: string; username: string; password: string }) =>
      unwrap(await api.accountYggdrasilLogin(v.apiRoot, v.username, v.password)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

export function useRemoveAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: AccountId) => unwrap(await api.accountRemove(id)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

export function useSetDefaultAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: AccountId) => unwrap(await api.accountSetDefault(id)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

export function useUpdateInstance() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { id: InstanceId; patch: InstancePatch }) =>
      unwrap(await api.instanceUpdate(v.id, v.patch)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.instances }),
  });
}

export function useDeleteInstance() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: InstanceId) => unwrap(await api.instanceDelete(id)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.instances }),
  });
}

export function useKillInstance() {
  return useMutation({
    mutationFn: async (id: InstanceId) => unwrap(await api.instanceKill(id)),
  });
}

export function useCancelTask() {
  return useMutation({
    mutationFn: async (id: TaskId) => unwrap(await api.taskCancel(id)),
  });
}

// ---- tasks -----------------------------------------------------------------

type ChannelArg = ReturnType<typeof createTaskChannel>;

export interface StartTaskOptions<V> {
  /** Calls the task command with the prepared channel. */
  run: (vars: V, channel: ChannelArg) => ReturnType<typeof api.instanceCreate>;
  /** Queries re-read after `finished` (results are never sent in events). */
  invalidate?: QueryKey[];
  onFinished?: (error: AppError | null, vars: V) => void;
}

/**
 * Starts a backend task: creates its `Channel<TaskEvent>`, feeds every event
 * into the task store and invalidates `invalidate` once it finishes.
 * The mutation resolves with the `TaskId` as soon as the command returns.
 */
export function useStartTask<V>(opts: StartTaskOptions<V>) {
  const qc = useQueryClient();
  const applyEvent = useTaskStore((s) => s.applyEvent);
  return useMutation({
    mutationFn: async (vars: V) => {
      const channel = createTaskChannel((ev: TaskEvent) => {
        applyEvent(ev);
        if (ev.type === "finished") {
          for (const key of opts.invalidate ?? []) void qc.invalidateQueries({ queryKey: key });
          opts.onFinished?.(ev.error, vars);
        }
      });
      return unwrap(await opts.run(vars, channel));
    },
  });
}
