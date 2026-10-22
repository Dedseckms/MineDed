// TanStack Query hooks over `api`. All backend access from components goes here.
import {
  type QueryKey,
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import type {
  AccountId,
  AppError,
  ContentFilter,
  ContentSourceKind,
  ContentVersionRef,
  InstalledContent,
  InstanceId,
  InstancePatch,
  LoaderKind,
  RoomAuthMode,
  SearchQuery,
  Settings,
  SkinId,
  SkinModel,
  TaskEvent,
  TaskId,
  VersionKind,
  VpnMode,
  VpnNode,
} from "../bindings";
import { nextOffset } from "../lib/content";
import { selectNode } from "../lib/vpn";
import { useRoomStore } from "../stores/roomStore";
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
  loaderVersions: (loader: LoaderKind, mc: string) => ["loaderVersions", loader, mc] as const,
  content: (id: InstanceId) => ["content", id] as const,
  search: (q: SearchQuery) => ["search", q] as const,
  contentVersions: (source: ContentSourceKind, projectId: string, filter: ContentFilter) =>
    ["contentVersions", source, projectId, filter] as const,
  skins: ["skins"] as const,
  capes: (account: AccountId) => ["capes", account] as const,
  vpnState: ["vpnState"] as const,
  vpnNodes: ["vpnNodes"] as const,
  room: ["room"] as const,
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

export function useYggdrasilRegister() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { apiRoot: string; invite: string; username: string; password: string }) =>
      unwrap(await api.accountYggdrasilRegister(v.apiRoot, v.invite, v.username, v.password)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

// ---- loaders & content -----------------------------------------------------

export const useLoaderVersions = (loader: LoaderKind, mc: string) =>
  useQuery({
    queryKey: qk.loaderVersions(loader, mc),
    queryFn: async () => unwrap(await api.loaderVersions(loader, mc)),
    enabled: loader !== "vanilla" && mc !== "",
    staleTime: 5 * 60_000,
  });

export const useContentList = (id: InstanceId) =>
  useQuery({ queryKey: qk.content(id), queryFn: async () => unwrap(await api.contentList(id)) });

/** Paged catalog search (infinite scroll); `query.offset` is ignored. */
export function useContentSearch(query: SearchQuery, enabled = true) {
  return useInfiniteQuery({
    queryKey: qk.search({ ...query, offset: 0 }),
    queryFn: async ({ pageParam }) => unwrap(await api.contentSearch({ ...query, offset: pageParam })),
    initialPageParam: 0,
    getNextPageParam: nextOffset,
    placeholderData: keepPreviousData,
    staleTime: 60_000,
    enabled,
  });
}

export const useContentVersions = (
  source: ContentSourceKind,
  projectId: string | null,
  filter: ContentFilter,
) =>
  useQuery({
    queryKey: qk.contentVersions(source, projectId ?? "", filter),
    queryFn: async () => unwrap(await api.contentVersions(source, projectId ?? "", filter)),
    enabled: projectId !== null,
    staleTime: 60_000,
  });

export function useResolveContent() {
  return useMutation({
    mutationFn: async (v: { instance: InstanceId; picks: ContentVersionRef[] }) =>
      unwrap(await api.contentResolve(v.instance, v.picks)),
  });
}

/** Toggles a file; the list is updated optimistically. */
export function useSetContentEnabled(instance: InstanceId) {
  const qc = useQueryClient();
  const key = qk.content(instance);
  return useMutation({
    mutationFn: async (v: { fileName: string; enabled: boolean }) =>
      unwrap(await api.contentSetEnabled(instance, v.fileName, v.enabled)),
    onMutate: async (v) => {
      await qc.cancelQueries({ queryKey: key });
      const prev = qc.getQueryData<InstalledContent[]>(key);
      qc.setQueryData<InstalledContent[]>(key, (list) =>
        list?.map((c) => (c.fileName === v.fileName ? { ...c, enabled: v.enabled } : c)),
      );
      return { prev };
    },
    onError: (_e, _v, ctx) => {
      if (ctx?.prev) qc.setQueryData(key, ctx.prev);
    },
    onSettled: () => qc.invalidateQueries({ queryKey: key }),
  });
}

export function useRemoveContent(instance: InstanceId) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (fileName: string) => unwrap(await api.contentRemove(instance, fileName)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.content(instance) }),
  });
}

// ---- skins -----------------------------------------------------------------

export const useSkins = () =>
  useQuery({ queryKey: qk.skins, queryFn: async () => unwrap(await api.skinLibrary()) });

export function useAddSkin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { path: string; name: string; model: SkinModel }) =>
      unwrap(await api.skinAdd(v.path, v.name, v.model)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.skins }),
  });
}

export function useRemoveSkin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: SkinId) => unwrap(await api.skinRemove(id)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.skins }),
  });
}

export function useApplySkin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { account: AccountId; skin: SkinId }) => unwrap(await api.skinApply(v.account, v.skin)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.accounts }),
  });
}

export const useCapes = (account: AccountId | null) =>
  useQuery({
    queryKey: qk.capes(account ?? ""),
    queryFn: async () => unwrap(await api.capeList(account ?? "")),
    enabled: account !== null,
  });

export function useSetCape() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { account: AccountId; capeId: string | null }) =>
      unwrap(await api.capeSet(v.account, v.capeId)),
    onSettled: (_d, _e, v) => qc.invalidateQueries({ queryKey: qk.capes(v.account) }),
  });
}

// ---- vpn -------------------------------------------------------------------

export const useVpnState = () =>
  useQuery({ queryKey: qk.vpnState, queryFn: async () => unwrap(await api.vpnState()) });

export const useVpnNodes = () =>
  useQuery({ queryKey: qk.vpnNodes, queryFn: async () => unwrap(await api.vpnNodes()) });

function useInvalidateVpn() {
  const qc = useQueryClient();
  return () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: qk.vpnState }),
      qc.invalidateQueries({ queryKey: qk.vpnNodes }),
    ]);
}

/** Replaces the subscription URL list (the refresh is a separate task). */
export function useSetVpnSubscription() {
  return useMutation({
    mutationFn: async (urls: string[]) => unwrap(await api.vpnSetSubscription(urls)),
  });
}

export function useImportVpnFile() {
  const invalidate = useInvalidateVpn();
  return useMutation({
    mutationFn: async (path: string) => unwrap(await api.vpnImportSubscriptionFile(path)),
    onSettled: invalidate,
  });
}

export function useSetVpnMode() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (mode: VpnMode) => unwrap(await api.vpnSetMode(mode)),
    onSuccess: (state) => qc.setQueryData(qk.vpnState, state),
    onError: () => qc.invalidateQueries({ queryKey: qk.vpnState }),
  });
}

/** Selects a node; the list is updated optimistically. */
export function useSelectVpnNode() {
  const qc = useQueryClient();
  const invalidate = useInvalidateVpn();
  return useMutation({
    mutationFn: async (node: string) => unwrap(await api.vpnSelect(node)),
    onMutate: async (node) => {
      await qc.cancelQueries({ queryKey: qk.vpnNodes });
      const prev = qc.getQueryData<VpnNode[]>(qk.vpnNodes);
      qc.setQueryData<VpnNode[]>(qk.vpnNodes, (list) => (list ? selectNode(list, node) : list));
      return { prev };
    },
    onError: (_e, _node, ctx) => {
      if (ctx?.prev) qc.setQueryData(qk.vpnNodes, ctx.prev);
    },
    onSettled: invalidate,
  });
}

export function useTestVpnLatency() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => unwrap(await api.vpnTestLatency()),
    onSuccess: (nodes) => qc.setQueryData(qk.vpnNodes, nodes),
  });
}

// ---- rooms & chat ----------------------------------------------------------

export const useRoomState = () =>
  useQuery({ queryKey: qk.room, queryFn: async () => unwrap(await api.roomState()) });

export function useCreateRoom() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { instance: InstanceId; authMode: RoomAuthMode; password: string | null }) =>
      unwrap(await api.roomCreate(v.instance, v.authMode, v.password)),
    onSuccess: (state) => qc.setQueryData(qk.room, state),
  });
}

export function useJoinRoom() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (v: { code: string; instance: InstanceId; password: string | null }) =>
      unwrap(await api.roomJoin(v.code, v.instance, v.password)),
    onSuccess: (state) => qc.setQueryData(qk.room, state),
  });
}

export function useLeaveRoom() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => unwrap(await api.roomLeave()),
    onSuccess: () => qc.setQueryData(qk.room, null),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.room }),
  });
}

export function useKickPeer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (peerId: string) => unwrap(await api.roomKick(peerId)),
    onSettled: () => qc.invalidateQueries({ queryKey: qk.room }),
  });
}

/** Sends a chat line; the returned message goes straight into the history. */
export function useSendChat() {
  const addOwnMessage = useRoomStore((s) => s.addOwnMessage);
  return useMutation({
    mutationFn: async (text: string) => unwrap(await api.chatSend(text)),
    onSuccess: addOwnMessage,
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
