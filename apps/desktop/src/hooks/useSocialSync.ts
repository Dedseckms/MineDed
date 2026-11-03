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
import { forgetGuest, savedGuestNick } from "../lib/guestSession";
import { canModerate } from "../lib/social";
import { playCue } from "../lib/sound";
import { usePrefsStore } from "../stores/prefsStore";

/**
 * Sound for an incoming message: silent for the own messages, for players the
 * user muted and while the message's channel is the one on screen. Staff
 * (admins and moderators) can never be silenced.
 */
export function shouldPlayMessageSound(
  event: SocialEvent,
  selfUuid: string,
  openChannel: string,
): boolean {
  if (event.type !== "message") return false;
  const author = event.message.author;
  if (author.uuid === selfUuid) return false;
  if (openChannel !== "" && event.message.channel === openChannel) return false;
  const prefs = usePrefsStore.getState();
  if (!prefs.chatSound) return false;
  if (canModerate(author.role)) return true;
  return !prefs.mutedChat.includes(author.uuid);
}

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
      const state = useSocialStore.getState();
      if (shouldPlayMessageSound(event, state.self?.uuid ?? "", state.selected)) playCue("message");
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
        const existing = await api.socialSession().catch(() => null);
        if (!alive) return;
        const current = existing?.status === "ok" ? existing.data : null;
        if (!account) {
          // No server account: keep a live guest session, drop everything else.
          if (current?.guest === true) {
            useSocialStore
              .getState()
              .set({ self: current, connected: true, error: null });
            return;
          }
          const nick = savedGuestNick();
          if (nick) {
            try {
              const user = unwrap(await api.socialGuestLogin(nick));
              if (alive)
                useSocialStore
                  .getState()
                  .set({ self: user, connected: true, error: null });
              return;
            } catch {
              forgetGuest();
            }
          }
          await api.socialDisconnect();
          useSocialStore
            .getState()
            .set({ self: null, connected: false, error: null });
          return;
        }
        // A server account on this device always wins over a leftover guest
        // session: `connect` swaps the stored guest for the account.
        try {
          const user = unwrap(await api.socialConnect(account));
          if (!alive) return;
          useSocialStore
            .getState()
            .set({ self: user, connected: true, error: null });
        } catch (error) {
          if (!alive) return;
          // e.g. an account of a third-party server: say why, do not stay silent.
          useSocialStore.getState().set({ self: null, error });
        }
      });
    return () => {
      alive = false;
      off();
      connection = connection
        .catch(() => {})
        .then(async () => {
          // Leave a guest session running: it is not tied to the game account.
          const current = await api.socialSession().catch(() => null);
          if (current?.status === "ok" && current.data?.guest === true) return;
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
