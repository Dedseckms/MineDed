import { create } from "zustand";

import type { RoomId, VoiceSpeakingPayload, VoiceStatus } from "../bindings";
import { SELF_PEER_ID, type SpeakingInfo, applySpeaking } from "../lib/voice";

/**
 * Live voice indicators (`voice://speaking`) and local peer mutes. Session
 * state itself (active, self mute, mode, mic test) comes from `voice_status`
 * via TanStack Query (`useVoiceStatus`).
 */
interface VoiceState {
  roomId: RoomId | null;
  /** Peers muted locally by this user. */
  mutedPeers: Record<string, boolean>;
  speaking: Record<string, SpeakingInfo>;
  /** Local microphone level 0..1 (`peerId === "self"`). */
  selfLevel: number;
  syncRoom: (roomId: RoomId | null) => void;
  setPeerMuted: (peerId: string, muted: boolean) => void;
  applySpeaking: (ev: VoiceSpeakingPayload) => void;
  /** Clears stale indicators when voice / mic test stops. */
  applyStatus: (status: VoiceStatus) => void;
  reset: () => void;
}

const session = { mutedPeers: {}, speaking: {}, selfLevel: 0 };
const initial = { roomId: null, ...session };

export const useVoiceStore = create<VoiceState>()((set) => ({
  ...initial,
  syncRoom: (roomId) => set((s) => (s.roomId === roomId ? s : { roomId, ...session })),
  setPeerMuted: (peerId, muted) =>
    set((s) => {
      const mutedPeers = { ...s.mutedPeers };
      if (muted) mutedPeers[peerId] = true;
      else delete mutedPeers[peerId];
      return { mutedPeers };
    }),
  applySpeaking: (ev) =>
    set((s) => {
      const speaking = applySpeaking(s.speaking, ev);
      if (ev.peerId === SELF_PEER_ID) {
        const level = Math.min(1, Math.max(0, ev.level ?? (ev.speaking ? 1 : 0)));
        return speaking === s.speaking && level === s.selfLevel ? s : { speaking, selfLevel: level };
      }
      return speaking === s.speaking ? s : { speaking };
    }),
  applyStatus: (status) =>
    set((s) => {
      if (status.active) return s;
      const speaking = Object.keys(s.speaking).length === 0 ? s.speaking : {};
      const selfLevel = status.micTest ? s.selfLevel : 0;
      return speaking === s.speaking && selfLevel === s.selfLevel ? s : { speaking, selfLevel };
    }),
  reset: () => set(initial),
}));
