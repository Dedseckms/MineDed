import { create } from "zustand";

import type { RoomId, VoiceSpeakingPayload } from "../bindings";
import { SELF_PEER_ID, type SpeakingInfo, applySpeaking } from "../lib/voice";

/**
 * Local voice session state. The backend has no `voice_state` getter yet, so
 * what this client asked for (active, self mute, local peer mutes) is kept here.
 */
interface VoiceState {
  roomId: RoomId | null;
  active: boolean;
  selfMuted: boolean;
  /** Peers muted locally by this user. */
  mutedPeers: Record<string, boolean>;
  speaking: Record<string, SpeakingInfo>;
  /** Local microphone level 0..1 (`peerId === "self"`). */
  selfLevel: number;
  micTest: boolean;
  syncRoom: (roomId: RoomId | null) => void;
  setActive: (active: boolean) => void;
  setSelfMuted: (muted: boolean) => void;
  setPeerMuted: (peerId: string, muted: boolean) => void;
  setMicTest: (on: boolean) => void;
  applySpeaking: (ev: VoiceSpeakingPayload) => void;
  reset: () => void;
}

const session = { active: false, selfMuted: false, mutedPeers: {}, speaking: {}, selfLevel: 0 };
const initial = { roomId: null, micTest: false, ...session };

export const useVoiceStore = create<VoiceState>()((set) => ({
  ...initial,
  syncRoom: (roomId) => set((s) => (s.roomId === roomId ? s : { roomId, ...session })),
  setActive: (active) => set(active ? { active } : { active, speaking: {}, selfLevel: 0 }),
  setSelfMuted: (selfMuted) => set({ selfMuted }),
  setPeerMuted: (peerId, muted) =>
    set((s) => {
      const mutedPeers = { ...s.mutedPeers };
      if (muted) mutedPeers[peerId] = true;
      else delete mutedPeers[peerId];
      return { mutedPeers };
    }),
  setMicTest: (micTest) => set(micTest ? { micTest } : { micTest, selfLevel: 0 }),
  applySpeaking: (ev) =>
    set((s) => {
      if (ev.peerId === SELF_PEER_ID) {
        const level = Math.min(1, Math.max(0, ev.level ?? (ev.speaking ? 1 : 0)));
        const speaking = applySpeaking(s.speaking, ev);
        return speaking === s.speaking && level === s.selfLevel ? s : { speaking, selfLevel: level };
      }
      const speaking = applySpeaking(s.speaking, ev);
      return speaking === s.speaking ? s : { speaking };
    }),
  reset: () => set(initial),
}));
