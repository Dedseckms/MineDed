import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it } from "vitest";

import type { ChatMessage, RoomState } from "../bindings";
import { qk } from "../api/queries";
import { applyRoomPeer, applyRoomState, notifyNetBlocked } from "../hooks/useBackendSync";
import { useNavStore } from "./navStore";
import { useRoomStore } from "./roomStore";
import { TOAST_LIMIT, useToastStore } from "./toastStore";

const msg = (id: string, fromPeer = "other"): ChatMessage => ({
  id,
  fromPeer,
  fromName: fromPeer,
  text: id,
  sentAt: "2026-10-22T10:00:00Z",
  viaRelay: false,
});

const room = (roomId: string): RoomState => ({
  roomId,
  code: "ABCD-1234",
  role: "host",
  authMode: "open",
  sig: { mc: "1.21.1", loader: "vanilla", loaderVersion: null, modsHash: "0" },
  peers: [],
  expiresAt: "2026-10-22T12:00:00Z",
  link: "mineup://join/ABCD-1234",
  passwordProtected: false,
  localAddr: null,
});

beforeEach(() => {
  useRoomStore.getState().reset();
  useToastStore.getState().reset();
  useNavStore.getState().go("play");
});

describe("roomStore", () => {
  it("counts unread only for unseen foreign messages and dedupes echoes", () => {
    const s = useRoomStore.getState();
    s.syncRoom("r1");
    s.addMessage(msg("1"), false);
    s.addMessage(msg("1"), false);
    s.addMessage(msg("2"), true);
    expect(useRoomStore.getState().unread).toBe(1);
    s.addOwnMessage(msg("3", "me"));
    s.addMessage(msg("3", "me"), false); // own echo
    s.addMessage(msg("4", "me"), false); // own message from another window
    expect(useRoomStore.getState().messages.map((m) => m.id)).toEqual(["1", "2", "3", "4"]);
    expect(useRoomStore.getState().unread).toBe(1);
    s.markRead();
    expect(useRoomStore.getState().unread).toBe(0);
  });

  it("clears history when the room changes, keeps it for the same room", () => {
    const s = useRoomStore.getState();
    s.syncRoom("r1");
    s.addOwnMessage(msg("1", "me"));
    s.syncRoom("r1");
    expect(useRoomStore.getState().messages).toHaveLength(1);
    s.syncRoom(null);
    expect(useRoomStore.getState()).toMatchObject({ roomId: null, messages: [], selfPeerId: null });
  });
});

describe("toastStore", () => {
  it("caps the stack and replaces toasts with the same dedupe key", () => {
    const s = useToastStore.getState();
    for (let i = 0; i < TOAST_LIMIT + 2; i += 1) s.push({ tone: "info", messageKey: `k${i}` });
    expect(useToastStore.getState().toasts.map((t) => t.messageKey)).toEqual(["k2", "k3", "k4"]);
    s.push({ tone: "warning", messageKey: "a", dedupeKey: "x" });
    const id = s.push({ tone: "warning", messageKey: "b", dedupeKey: "x" });
    expect(useToastStore.getState().toasts.filter((t) => t.dedupeKey === "x").map((t) => t.messageKey)).toEqual(["b"]);
    s.dismiss(id);
    expect(useToastStore.getState().toasts.some((t) => t.id === id)).toBe(false);
  });
});

describe("event appliers", () => {
  it("net://blocked → one toast per host with a VPN shortcut", () => {
    notifyNetBlocked({ host: "cdn.example", suggestionKey: "net.blockedHint" });
    notifyNetBlocked({ host: "cdn.example", suggestionKey: "net.blockedHint" });
    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({ messageKey: "net.blocked", params: { host: "cdn.example" } });
    toasts[0]?.action?.run();
    expect(useNavStore.getState().section).toBe("vpn");
    notifyNetBlocked({ host: "other.example", suggestionKey: "" });
    expect(useToastStore.getState().toasts.at(-1)).toMatchObject({ detailKey: "net.blockedHint", action: undefined });
  });

  it("room://state and room://peer update the cache and the chat room", () => {
    const qc = new QueryClient();
    applyRoomState(qc, room("r1"));
    expect(useRoomStore.getState().roomId).toBe("r1");
    applyRoomPeer(qc, {
      joined: true,
      peer: { peerId: "p", name: "P", uuid: null, path: "relay", rttMs: 80, speaking: false, muted: false },
    });
    expect(qc.getQueryData<RoomState>(qk.room)?.peers).toHaveLength(1);
    applyRoomState(qc, null);
    expect(qc.getQueryData(qk.room)).toBeNull();
    expect(useRoomStore.getState().roomId).toBeNull();
  });
});
