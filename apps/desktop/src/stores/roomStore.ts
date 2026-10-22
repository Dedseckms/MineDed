import { create } from "zustand";

import type { ChatMessage, RoomId } from "../bindings";
import { appendChat } from "../lib/rooms";

interface RoomChatState {
  /** Room the chat history belongs to (`null` = not in a room). */
  roomId: RoomId | null;
  messages: ChatMessage[];
  /** Messages received while the multiplayer screen was not visible. */
  unread: number;
  /** Own peer id, learned from the `chat_send` reply (marks own messages). */
  selfPeerId: string | null;
  /** Called on every room state change; switching rooms clears the history. */
  syncRoom: (roomId: RoomId | null) => void;
  /** `seen` = the user is looking at the chat (no unread counter). */
  addMessage: (msg: ChatMessage, seen: boolean) => void;
  /** Adds a message this client sent. */
  addOwnMessage: (msg: ChatMessage) => void;
  markRead: () => void;
  reset: () => void;
}

const initial = { roomId: null, messages: [], unread: 0, selfPeerId: null };

export const useRoomStore = create<RoomChatState>()((set) => ({
  ...initial,
  syncRoom: (roomId) =>
    set((s) => (s.roomId === roomId ? s : { roomId, messages: [], unread: 0, selfPeerId: null })),
  addMessage: (msg, seen) =>
    set((s) => {
      const messages = appendChat(s.messages, msg);
      if (messages === s.messages) return s;
      const own = s.selfPeerId !== null && msg.fromPeer === s.selfPeerId;
      return { messages, unread: seen || own ? s.unread : s.unread + 1 };
    }),
  addOwnMessage: (msg) => set((s) => ({ selfPeerId: msg.fromPeer, messages: appendChat(s.messages, msg) })),
  markRead: () => set((s) => (s.unread === 0 ? s : { unread: 0 })),
  reset: () => set(initial),
}));
