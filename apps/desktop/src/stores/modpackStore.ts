import { create } from "zustand";

import type { SortBy } from "../bindings";
import { DEFAULT_MODPACK_FILTERS, type ModpackFilters, type ModpackLoader } from "../lib/modpacks";

interface ModpackState extends ModpackFilters {
  setText: (text: string) => void;
  setMc: (mc: string | null) => void;
  setLoader: (loader: ModpackLoader | null) => void;
  setSort: (sort: SortBy) => void;
  reset: () => void;
}

/** Modpack catalog filters (Modrinth only); kept across tab switches and instance pages. */
export const useModpackStore = create<ModpackState>()((set) => ({
  ...DEFAULT_MODPACK_FILTERS,
  setText: (text) => set({ text }),
  setMc: (mc) => set({ mc }),
  setLoader: (loader) => set({ loader }),
  setSort: (sort) => set({ sort }),
  reset: () => set(DEFAULT_MODPACK_FILTERS),
}));

export const selectModpackFilters = (s: ModpackState): ModpackFilters => ({
  source: s.source,
  text: s.text,
  mc: s.mc,
  loader: s.loader,
  sort: s.sort,
});
