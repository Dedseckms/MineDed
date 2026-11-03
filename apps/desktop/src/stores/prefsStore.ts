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
  /** Short sound for every incoming chat message. */
  chatSound: boolean;
  setChatSound: (on: boolean) => void;
  /** Players whose messages stay silent (staff cannot be muted). */
  mutedChat: string[];
  toggleMutedChat: (uuid: string) => void;
}

export const usePrefsStore = create<PrefsState>()(
  persist(
    (set) => ({
      pttSound: true,
      setPttSound: (pttSound) => set({ pttSound }),
      autoHostPack: false,
      setAutoHostPack: (autoHostPack) => set({ autoHostPack }),
      chatSound: true,
      setChatSound: (chatSound) => set({ chatSound }),
      mutedChat: [],
      toggleMutedChat: (uuid) =>
        set((s) => ({
          mutedChat: s.mutedChat.includes(uuid)
            ? s.mutedChat.filter((x) => x !== uuid)
            : [...s.mutedChat, uuid],
        })),
    }),
    {
      name: "mineup.ui-prefs",
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({
        pttSound: s.pttSound,
        autoHostPack: s.autoHostPack,
        chatSound: s.chatSound,
        mutedChat: s.mutedChat,
      }),
    },
  ),
);
