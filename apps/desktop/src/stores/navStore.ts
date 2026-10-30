import { create } from "zustand";

import type { AccountId, InstanceId } from "../bindings";

export const SECTIONS = ["play", "instances", "multiplayer", "vpn", "chat", "skins", "accounts", "settings"] as const;
export type Section = (typeof SECTIONS)[number];

export const INSTANCE_TABS = ["mods", "resourcePacks", "shaders", "browse"] as const;
export type InstanceTab = (typeof INSTANCE_TABS)[number];

/** Tabs of the instances library: own instances and the modpack catalog. */
export const LIBRARY_TABS = ["mine", "catalog"] as const;
export type LibraryTab = (typeof LIBRARY_TABS)[number];

interface NavState {
  section: Section;
  /** Selection on the Play screen (`null` = pick automatically). */
  playInstance: InstanceId | null;
  playAccount: AccountId | null;
  /** Instance page opened from the library (`null` = library list). */
  openInstance: InstanceId | null;
  instanceTab: InstanceTab;
  libraryTab: LibraryTab;
  go: (section: Section) => void;
  setLibraryTab: (tab: LibraryTab) => void;
  selectPlayInstance: (id: InstanceId | null) => void;
  selectPlayAccount: (id: AccountId | null) => void;
  showInstance: (id: InstanceId | null, tab?: InstanceTab) => void;
  setInstanceTab: (tab: InstanceTab) => void;
  /** Deep link `join/<code>`: prefill for the Multiplayer join form (consumed once). */
  pendingJoin: string | null;
  /** Deep link `invite/<code>`: prefill for the own-server registration form (consumed once). */
  pendingInvite: string | null;
  openJoin: (code: string) => void;
  openInvite: (code: string) => void;
  takeJoin: () => string | null;
  takeInvite: () => string | null;
}

export const useNavStore = create<NavState>()((set, get) => ({
  section: "play",
  playInstance: null,
  playAccount: null,
  openInstance: null,
  instanceTab: "mods",
  libraryTab: "mine",
  go: (section) => set({ section, openInstance: null }),
  setLibraryTab: (libraryTab) => set({ libraryTab }),
  selectPlayInstance: (playInstance) => set({ playInstance }),
  selectPlayAccount: (playAccount) => set({ playAccount }),
  showInstance: (openInstance, tab) =>
    set((s) => ({ section: "instances", openInstance, instanceTab: tab ?? (openInstance === s.openInstance ? s.instanceTab : "mods") })),
  setInstanceTab: (instanceTab) => set({ instanceTab }),
  pendingJoin: null,
  pendingInvite: null,
  openJoin: (pendingJoin) => set({ section: "multiplayer", openInstance: null, pendingJoin }),
  openInvite: (pendingInvite) => set({ section: "accounts", openInstance: null, pendingInvite }),
  takeJoin: () => {
    const v = get().pendingJoin;
    if (v !== null) set({ pendingJoin: null });
    return v;
  },
  takeInvite: () => {
    const v = get().pendingInvite;
    if (v !== null) set({ pendingInvite: null });
    return v;
  },
}));
