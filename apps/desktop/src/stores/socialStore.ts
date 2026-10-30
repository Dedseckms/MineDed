import { create } from "zustand";
import type { SocialUser } from "../bindings";

interface SocialState {
  self: SocialUser | null;
  connected: boolean;
  error: unknown;
  unread: number;
  selected: string;
  blocked: string[];
  set: (
    patch: Partial<
      Pick<SocialState, "self" | "connected" | "error" | "unread" | "selected">
    >,
  ) => void;
  block: (uuid: string, value: boolean) => void;
  reset: () => void;
}
const initial = {
  self: null,
  connected: false,
  error: null,
  unread: 0,
  selected: "global",
  blocked: [],
};
export const useSocialStore = create<SocialState>()((set) => ({
  ...initial,
  set: (patch) => set(patch),
  block: (uuid, value) =>
    set((s) => ({
      blocked: value
        ? [...new Set([...s.blocked, uuid])]
        : s.blocked.filter((id) => id !== uuid),
    })),
  reset: () => set(initial),
}));
