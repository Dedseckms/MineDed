import { useEffect } from "react";
import {
  useQuery,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from "@tanstack/react-query";
import type { SocialEvent, SocialHistory } from "../bindings";
import { api, unwrap } from "../api/client";
import { subscribe } from "../api/events";
import { useAccounts, useRoomState } from "../api/queries";
import { useSocialStore } from "../stores/socialStore";
import { useGameStore } from "../stores/gameStore";

/** Update loaded pages in place; incoming traffic must not refetch an entire cloud history. */
export function applySocialEvent(qc: QueryClient, event: SocialEvent) {
  if (event.type === "message" || event.type === "messageUpdated") {
    const message = event.message;
    qc.setQueryData<InfiniteData<SocialHistory>>(
      ["social", "history", message.channel],
      (data) => {
        if (!data) return data;
        const exists = data.pages.some((page) =>
          page.messages.some((item) => item.id === message.id),
        );
        return {
          ...data,
          pages: data.pages.map((page, index) => ({
            ...page,
            messages:
              !exists && index === 0 && event.type === "message"
                ? [...page.messages, message]
                : page.messages.map((item) =>
                    item.id === message.id ? message : item,
                  ),
          })),
        };
      },
    );
    void qc.invalidateQueries({ queryKey: ["social", "channels"] });
  } else if (event.type === "admin") {
    qc.setQueryData(["social", "server"], event.state);
  } else if (event.type === "connection") {
    if (event.connected) void qc.invalidateQueries({ queryKey: ["social"] });
  } else if (event.type === "friend" || event.type === "friendRemoved") {
    void qc.invalidateQueries({ queryKey: ["social", "friends"] });
    void qc.invalidateQueries({ queryKey: ["social", "profile"] });
  } else if (event.type === "presence") {
    void qc.invalidateQueries({ queryKey: ["social", "profile", event.uuid] });
    void qc.invalidateQueries({ queryKey: ["social", "channels"] });
    void qc.invalidateQueries({ queryKey: ["social", "friends"] });
  }
}

let connection: Promise<void> = Promise.resolve();

/** One shell-owned connection; account changes clear every private cache. */
export function useSocialSync() {
  const accounts = useAccounts();
  const current = accounts.data?.find((a) => a.isDefault) ?? accounts.data?.[0];
  const account =
    current?.kind === "yggdrasil" && current.valid ? current.id : null;
  const qc = useQueryClient();
  const room = useRoomState();
  const running = useGameStore((s) =>
    Object.values(s.runs).some((v) => v.state === "running"),
  );
  const self = useSocialStore((s) => s.self);
  const channels = useQuery({
    queryKey: ["social", "channels"],
    queryFn: async () => unwrap(await api.socialChannels()),
    enabled: !!self,
    refetchInterval: 30000,
  });
  useEffect(() => {
    useSocialStore
      .getState()
      .set({ unread: channels.data?.reduce((n, c) => n + c.unread, 0) ?? 0 });
  }, [channels.data]);
  useEffect(() => {
    let alive = true;
    useSocialStore.getState().reset();
    void qc.cancelQueries({ queryKey: ["social"] });
    qc.removeQueries({ queryKey: ["social"] });
    const off = subscribe("socialEvent", (event) => {
      if (!alive) return;
      if (event.type === "connection")
        useSocialStore.getState().set({ connected: event.connected });
      if (event.type === "rejected")
        useSocialStore.getState().set({
          error: {
            code: "permissionDenied",
            message: event.reason,
            details: null,
          },
        });
      applySocialEvent(qc, event);
    });
    // Serialize session operations across StrictMode and account switches.
    connection = connection
      .catch(() => {})
      .then(async () => {
        await api.socialDisconnect();
        if (!alive || !account) return;
        try {
          const user = unwrap(await api.socialConnect(account));
          if (alive)
            useSocialStore
              .getState()
              .set({ self: user, connected: true, error: null });
        } catch (error) {
          if (alive) useSocialStore.getState().set({ error });
        }
      });
    return () => {
      alive = false;
      off();
      connection = connection
        .catch(() => {})
        .then(async () => {
          await api.socialDisconnect();
        });
    };
  }, [account, qc]);
  useEffect(() => {
    if (self)
      void api.socialSetPresence(
        room.data ? "inRoom" : running ? "inGame" : "online",
      );
  }, [self, running, room.data?.roomId]);
}
