import { create } from "zustand";

import type { InstanceId, RoomId } from "../bindings";

export type HostPackPhase = "idle" | "matching" | "needPack" | "downloading" | "attached";

/**
 * Game room guest: which local instance the joined room is bound to. Lives
 * outside the room view so switching sections does not re-run auto-matching.
 */
interface HostPackState {
  roomId: RoomId | null;
  phase: HostPackPhase;
  instance: InstanceId | null;
  /** Starts tracking `roomId` (no-op when already tracked). */
  begin: (roomId: RoomId) => boolean;
  setPhase: (roomId: RoomId, phase: HostPackPhase) => void;
  attach: (roomId: RoomId, instance: InstanceId) => void;
  reset: () => void;
}

export const useHostPackStore = create<HostPackState>()((set, get) => ({
  roomId: null,
  phase: "idle",
  instance: null,
  begin: (roomId) => {
    if (get().roomId === roomId) return false;
    set({ roomId, phase: "matching", instance: null });
    return true;
  },
  setPhase: (roomId, phase) => {
    if (get().roomId === roomId) set({ phase });
  },
  attach: (roomId, instance) => {
    if (get().roomId === roomId) set({ phase: "attached", instance });
  },
  reset: () => set({ roomId: null, phase: "idle", instance: null }),
}));
