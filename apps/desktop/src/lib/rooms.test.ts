import { describe, expect, it } from "vitest";

import type { ChatMessage, Peer, RoomState } from "../bindings";
import {
  CHAT_MAX,
  appendChat,
  applyPeerEvent,
  chatTime,
  parseJoinInput,
  passwordOrNull,
  validateChat,
  validateRoomPassword,
} from "./rooms";

const peer = (id: string, patch: Partial<Peer> = {}): Peer => ({
  peerId: id,
  name: id,
  uuid: null,
  path: "connecting",
  rttMs: null,
  speaking: false,
  muted: false,
  ...patch,
});

const room = (peers: Peer[]): RoomState => ({
  roomId: "r1",
  code: "ABCD-1234",
  role: "host",
  authMode: "open",
  sig: { mc: "1.21.1", loader: "fabric", loaderVersion: "0.16.14", modsHash: "0" },
  peers,
  expiresAt: "2026-10-22T12:00:00Z",
  link: "mineup://join/ABCD-1234",
  passwordProtected: false,
  localAddr: null,
});

const msg = (id: string): ChatMessage => ({
  id,
  fromPeer: "p",
  fromName: "P",
  text: id,
  sentAt: "2026-10-22T10:05:00Z",
  viaRelay: false,
});

describe("parseJoinInput", () => {
  it("normalises codes and links", () => {
    expect(parseJoinInput("  abcd-1234 ")).toEqual({ ok: true, code: "ABCD-1234" });
    expect(parseJoinInput("mineup://join/abcd-1234")).toEqual({ ok: true, code: "ABCD-1234" });
    expect(parseJoinInput("MINEUP://join/WXYZ-9876/?x=1")).toEqual({ ok: true, code: "WXYZ-9876" });
    expect(parseJoinInput("mineup://join/ab%2Dcd")).toEqual({ ok: true, code: "AB-CD" });
  });

  it("rejects empty, foreign links and malformed codes", () => {
    expect(parseJoinInput("")).toEqual({ ok: false, error: "validation.required" });
    expect(parseJoinInput("https://example.com/join/ABCD")).toEqual({ ok: false, error: "validation.room.link" });
    expect(parseJoinInput("mineup://join/%E0")).toEqual({ ok: false, error: "validation.room.link" });
    expect(parseJoinInput("ab")).toEqual({ ok: false, error: "validation.room.code" });
    expect(parseJoinInput("ABCD 1234")).toEqual({ ok: false, error: "validation.room.code" });
    expect(parseJoinInput("ABCD--1234")).toEqual({ ok: false, error: "validation.room.code" });
    expect(parseJoinInput("mineup://join/")).toEqual({ ok: false, error: "validation.room.code" });
  });
});

describe("passwords and chat", () => {
  it("room password is optional, 4–64 chars", () => {
    expect(validateRoomPassword("")).toBeNull();
    expect(validateRoomPassword("abc")).toBe("validation.room.password");
    expect(validateRoomPassword("abcd")).toBeNull();
    expect(validateRoomPassword("x".repeat(65))).toBe("validation.room.password");
    expect(passwordOrNull("")).toBeNull();
    expect(passwordOrNull(" a ")).toBe(" a ");
  });

  it("chat text must be non-blank and short", () => {
    expect(validateChat("   ")).toBe("validation.required");
    expect(validateChat("hi")).toBeNull();
    expect(validateChat("x".repeat(CHAT_MAX + 1))).toBe("validation.chat.length");
  });

  it("formats chat time and tolerates bad input", () => {
    expect(chatTime("not a date")).toBe("");
    expect(chatTime("2026-10-22T10:05:00Z")).toMatch(/^\d{2}:\d{2}$/);
  });
});

describe("applyPeerEvent", () => {
  it("adds, updates in place and removes peers", () => {
    let s: RoomState | null = room([peer("a")]);
    s = applyPeerEvent(s, { peer: peer("b"), joined: true });
    expect(s?.peers.map((p) => p.peerId)).toEqual(["a", "b"]);
    s = applyPeerEvent(s, { peer: peer("a", { path: "direct", rttMs: 20 }), joined: true });
    expect(s?.peers.map((p) => [p.peerId, p.path])).toEqual([
      ["a", "direct"],
      ["b", "connecting"],
    ]);
    s = applyPeerEvent(s, { peer: peer("a"), joined: false });
    expect(s?.peers.map((p) => p.peerId)).toEqual(["b"]);
  });

  it("is a no-op without a room or for unknown leavers", () => {
    expect(applyPeerEvent(null, { peer: peer("a"), joined: true })).toBeNull();
    expect(applyPeerEvent(undefined, { peer: peer("a"), joined: true })).toBeNull();
    const s = room([peer("a")]);
    expect(applyPeerEvent(s, { peer: peer("zzz"), joined: false })).toBe(s);
  });
});

describe("appendChat", () => {
  it("dedupes by id and keeps the newest messages", () => {
    const one = appendChat([], msg("1"));
    expect(appendChat(one, msg("1"))).toBe(one);
    let list = one;
    for (let i = 2; i <= 5; i += 1) list = appendChat(list, msg(String(i)), 3);
    expect(list.map((m) => m.id)).toEqual(["3", "4", "5"]);
  });
});
