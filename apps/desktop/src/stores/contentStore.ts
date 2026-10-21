import { create } from "zustand";

import type { SortBy } from "../bindings";
import type { SearchFilters, SearchKind, SearchSource } from "../lib/content";

interface ContentState extends SearchFilters {
  setSource: (source: SearchSource) => void;
  setKind: (kind: SearchKind) => void;
  setText: (text: string) => void;
  setSort: (sort: SortBy) => void;
  setCompatibleOnly: (v: boolean) => void;
  /** Opens the catalog pre-filtered (e.g. "browse shaders"). */
  browse: (kind: SearchKind) => void;
  reset: () => void;
}

const initial: SearchFilters = {
  source: "modrinth",
  kind: "mod",
  text: "",
  sort: "relevance",
  compatibleOnly: true,
};

/** Catalog filters; survive tab switches within the session. */
export const useContentStore = create<ContentState>()((set) => ({
  ...initial,
  setSource: (source) => set({ source }),
  setKind: (kind) => set({ kind }),
  setText: (text) => set({ text }),
  setSort: (sort) => set({ sort }),
  setCompatibleOnly: (compatibleOnly) => set({ compatibleOnly }),
  browse: (kind) => set({ kind, text: "", compatibleOnly: true }),
  reset: () => set(initial),
}));

/** The filters part of the state (stable shape for query keys). */
export function selectFilters(s: SearchFilters): SearchFilters {
  return { source: s.source, kind: s.kind, text: s.text, sort: s.sort, compatibleOnly: s.compatibleOnly };
}
