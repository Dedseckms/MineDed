import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

/**
 * UI-only preferences that the backend does not need (kept in localStorage,
 * per machine). Anything the backend uses belongs in `Settings`.
 */
interface PrefsState {
  /** Short sound on push-to-talk press/release. */
  pttSound: boolean;
  setPttSound: (on: boolean) => void;
  /** Game room guest: download the host's pack without asking. */
  autoHostPack: boolean;
  setAutoHostPack: (on: boolean) => void;
}

export const usePrefsStore = create<PrefsState>()(
  persist(
    (set) => ({
      pttSound: true,
      setPttSound: (pttSound) => set({ pttSound }),
      autoHostPack: false,
      setAutoHostPack: (autoHostPack) => set({ autoHostPack }),
    }),
    {
      name: "mineup.ui-prefs",
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({ pttSound: s.pttSound, autoHostPack: s.autoHostPack }),
    },
  ),
);
