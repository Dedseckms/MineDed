import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";

import type { NetBlockedPayload, RoomPeerEvent, RoomState } from "../bindings";
import { api, unwrap } from "../api/client";
import { subscribe } from "../api/events";
import { qk, useRoomState } from "../api/queries";
import { applyPeerEvent } from "../lib/rooms";
import { useGameStore } from "../stores/gameStore";
import { useNavStore } from "../stores/navStore";
import { useRoomStore } from "../stores/roomStore";
import { useTaskStore } from "../stores/taskStore";
import { useToastStore } from "../stores/toastStore";
import { useVoiceStore } from "../stores/voiceStore";

/** Toast for `net://blocked`; one per host, with a shortcut to the VPN screen. */
export function notifyNetBlocked(p: NetBlockedPayload): void {
  useToastStore.getState().push({
    tone: "warning",
    messageKey: "net.blocked",
    params: { host: p.host },
    detailKey: p.suggestionKey || "net.blockedHint",
    dedupeKey: `blocked:${p.host}`,
    action:
      useNavStore.getState().section === "vpn"
        ? undefined
        : { labelKey: "net.openVpn", run: () => useNavStore.getState().go("vpn") },
  });
}

/** Applies room events to the query cache and the chat store. */
export function applyRoomState(qc: QueryClient, state: RoomState | null): void {
  qc.setQueryData(qk.room, state);
  useRoomStore.getState().syncRoom(state?.roomId ?? null);
  useVoiceStore.getState().syncRoom(state?.roomId ?? null);
}

/**
 * Room, chat and voice subscriptions. Used by the main shell and by the
 * compact overlay window (which has its own JS context and stores).
 * `chatSeen` decides whether an incoming message counts as unread.
 */
export function useRoomSync(chatSeen: () => boolean): void {
  const qc = useQueryClient();
  const room = useRoomState();
  const seenRef = useRef(chatSeen);
  seenRef.current = chatSeen;

  useEffect(() => {
    if (room.data === undefined) return;
    const id = room.data?.roomId ?? null;
    useRoomStore.getState().syncRoom(id);
    useVoiceStore.getState().syncRoom(id);
  }, [room.data]);

  useEffect(() => {
    const offs = [
      subscribe("roomState", (state) => applyRoomState(qc, state)),
      subscribe("roomPeer", (ev) => applyRoomPeer(qc, ev)),
      subscribe("chatMessage", (msg) => useRoomStore.getState().addMessage(msg, seenRef.current())),
      subscribe("voiceSpeaking", (ev) => useVoiceStore.getState().applySpeaking(ev)),
    ];
    return () => offs.forEach((off) => off());
  }, [qc]);
}

export function applyRoomPeer(qc: QueryClient, ev: RoomPeerEvent): void {
  qc.setQueryData<RoomState | null>(qk.room, (s) => applyPeerEvent(s, ev));
}

/**
 * App-wide subscriptions: task center (`tasks_list` + `tasks://changed`),
 * game state/log, account changes, VPN, rooms and chat. Mounted once in the shell.
 */
export function useBackendSync(): void {
  const qc = useQueryClient();
  const syncSummaries = useTaskStore((s) => s.syncSummaries);
  const applyState = useGameStore((s) => s.applyState);
  const appendLog = useGameStore((s) => s.appendLog);

  const tasks = useQuery({ queryKey: qk.tasks, queryFn: async () => unwrap(await api.tasksList()) });
  useRoomSync(() => useNavStore.getState().section === "multiplayer");

  useEffect(() => {
    if (tasks.data) syncSummaries(tasks.data);
  }, [tasks.data, syncSummaries]);

  useEffect(() => {
    const offs = [
      subscribe("tasksChanged", (list) => {
        qc.setQueryData(qk.tasks, list);
      }),
      subscribe("instanceState", (p) => {
        applyState(p);
        qc.setQueryData(qk.instanceState(p.id), p.state);
        // lastPlayed / playtime change around launches.
        if (p.state === "running" || p.state === "stopped" || p.state === "crashed")
          void qc.invalidateQueries({ queryKey: qk.instances });
      }),
      subscribe("instanceLog", appendLog),
      subscribe("accountChanged", (list) => {
        qc.setQueryData(qk.accounts, list);
      }),
      subscribe("vpnState", (state) => {
        qc.setQueryData(qk.vpnState, state);
        // Node selection / latency may have changed together with the state.
        void qc.invalidateQueries({ queryKey: qk.vpnNodes });
      }),
      subscribe("netBlocked", notifyNetBlocked),
    ];
    return () => offs.forEach((off) => off());
  }, [qc, applyState, appendLog]);
}
