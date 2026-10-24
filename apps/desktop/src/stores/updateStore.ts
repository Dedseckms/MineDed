import { create } from "zustand";

import type { TaskId, UpdateInfo } from "../bindings";

interface UpdateState {
  /** Newest known launcher update (`update_check` result or `update://available`). */
  available: UpdateInfo | null;
  /** Version the user postponed with "Later"; the banner stays hidden for it this session. */
  dismissed: string | null;
  /** The running/last `update_install` task. */
  installTask: TaskId | null;
  /** The startup check ran (once per session). */
  autoChecked: boolean;
  /** Applies a check result: `null` means "up to date". */
  setAvailable: (info: UpdateInfo | null) => void;
  dismiss: () => void;
  /** Shows the banner again (e.g. after a manual check). */
  undismiss: () => void;
  setInstallTask: (id: TaskId | null) => void;
  markAutoChecked: () => void;
  reset: () => void;
}

const initial = { available: null, dismissed: null, installTask: null, autoChecked: false };

export const useUpdateStore = create<UpdateState>()((set) => ({
  ...initial,
  setAvailable: (available) => set({ available }),
  dismiss: () => set((s) => ({ dismissed: s.available?.version ?? null })),
  undismiss: () => set({ dismissed: null }),
  setInstallTask: (installTask) => set({ installTask }),
  markAutoChecked: () => set({ autoChecked: true }),
  reset: () => set(initial),
}));

/** The update to show in the banner, or `null`. */
export function selectBannerUpdate(s: Pick<UpdateState, "available" | "dismissed">): UpdateInfo | null {
  return s.available && s.available.version !== s.dismissed ? s.available : null;
}
