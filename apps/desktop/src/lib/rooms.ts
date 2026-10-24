// Pure helpers for P2P rooms and chat (no React, no backend calls).
import type { Account, ChatMessage, PathKind, RoomAuthMode, RoomPeerEvent, RoomState } from "../bindings";
import type { ValidationKey } from "./validation";

export const ROOM_LINK_PREFIX = "mineup://join/";
export const ROOM_PASSWORD_MIN = 4;
export const ROOM_PASSWORD_MAX = 64;
export const CHAT_MAX = 500;
/** Chat messages kept in memory per room (older ones are dropped). */
export const CHAT_HISTORY = 200;

const CODE = /^[A-Z0-9]+(?:-[A-Z0-9]+)*$/;

export type JoinParse = { ok: true; code: string } | { ok: false; error: ValidationKey };

/**
 * Accepts a room code (`abcd-1234`, any case) or a `mineup://join/<code>` link
 * and returns the upper-case code passed to `room_join`.
 */
export function parseJoinInput(input: string): JoinParse {
  let s = input.trim();
  if (s.length === 0) return { ok: false, error: "validation.required" };
  if (/^[a-z][a-z0-9+.-]*:/i.test(s)) {
    if (!s.toLowerCase().startsWith(ROOM_LINK_PREFIX)) return { ok: false, error: "validation.room.link" };
    s = s.slice(ROOM_LINK_PREFIX.length).split(/[/?#]/)[0] ?? "";
    try {
      s = decodeURIComponent(s);
    } catch {
      return { ok: false, error: "validation.room.link" };
    }
  }
  const code = s.trim().toUpperCase();
  if (code.length < 4 || code.length > 32 || !CODE.test(code)) return { ok: false, error: "validation.room.code" };
  return { ok: true, code };
}

/** Optional room password: empty = none, otherwise 4–64 characters. */
export function validateRoomPassword(pw: string): ValidationKey | null {
  if (pw.length === 0) return null;
  if (pw.length < ROOM_PASSWORD_MIN || pw.length > ROOM_PASSWORD_MAX) return "validation.room.password";
  return null;
}

export function passwordOrNull(pw: string): string | null {
  return pw.length === 0 ? null : pw;
}

export function validateChat(text: string): ValidationKey | null {
  const t = text.trim();
  if (t.length === 0) return "validation.required";
  if (t.length > CHAT_MAX) return "validation.chat.length";
  return null;
}

export const PATH_TONE = {
  direct: "accent",
  relay: "warning",
  connecting: "neutral",
} as const satisfies Record<PathKind, string>;

/** Applies a `room://peer` event to the cached room state. */
export function applyPeerEvent(state: RoomState | null | undefined, ev: RoomPeerEvent): RoomState | null {
  if (!state) return state ?? null;
  const rest = state.peers.filter((p) => p.peerId !== ev.peer.peerId);
  if (!ev.joined) return rest.length === state.peers.length ? state : { ...state, peers: rest };
  const idx = state.peers.findIndex((p) => p.peerId === ev.peer.peerId);
  const peers = idx < 0 ? [...state.peers, ev.peer] : state.peers.map((p, i) => (i === idx ? ev.peer : p));
  return { ...state, peers };
}

/** Appends a chat message: ignores duplicates (own echo), keeps the last `limit`. */
export function appendChat(list: readonly ChatMessage[], msg: ChatMessage, limit = CHAT_HISTORY): ChatMessage[] {
  if (list.some((m) => m.id === msg.id)) return list as ChatMessage[];
  const next = [...list, msg];
  return next.length > limit ? next.slice(next.length - limit) : next;
}

/** `HH:MM` of an RFC 3339 timestamp in local time (empty on bad input). */
export function chatTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export interface AuthModeChoice {
  /** Pre-selected mode for the default account. */
  suggested: RoomAuthMode;
  /** Mode the backend rejects for this account (`authModeMismatch`), if any. */
  disabled: RoomAuthMode | null;
}

/**
 * Room auth mode that matches the host account: the integrated server runs in
 * online mode for Yggdrasil hosts and offline mode for offline hosts, so the
 * other option fails on the backend with `reason = authModeMismatch`.
 */
export function authModeFor(account: Pick<Account, "kind"> | null | undefined): AuthModeChoice {
  if (!account) return { suggested: "open", disabled: null };
  return account.kind === "yggdrasil"
    ? { suggested: "yggdrasil", disabled: "open" }
    : { suggested: "open", disabled: "yggdrasil" };
}

/** The user's pick unless it is incompatible with the account. */
export function effectiveAuthMode(choice: AuthModeChoice, picked: RoomAuthMode | null): RoomAuthMode {
  return picked !== null && picked !== choice.disabled ? picked : choice.suggested;
}
