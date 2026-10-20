import { create } from "zustand";

import type { AccountId, InstanceId } from "../bindings";

export const SECTIONS = ["play", "instances", "accounts", "settings"] as const;
export type Section = (typeof SECTIONS)[number];

interface NavState {
  section: Section;
  /** Selection on the Play screen (`null` = pick automatically). */
  playInstance: InstanceId | null;
  playAccount: AccountId | null;
  go: (section: Section) => void;
  selectPlayInstance: (id: InstanceId | null) => void;
  selectPlayAccount: (id: AccountId | null) => void;
}

export const useNavStore = create<NavState>()((set) => ({
  section: "play",
  playInstance: null,
  playAccount: null,
  go: (section) => set({ section }),
  selectPlayInstance: (playInstance) => set({ playInstance }),
  selectPlayAccount: (playAccount) => set({ playAccount }),
}));
