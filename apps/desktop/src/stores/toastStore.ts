import { create } from "zustand";

export type ToastTone = "info" | "warning" | "danger";

export interface ToastAction {
  /** i18n key of the button label. */
  labelKey: string;
  run: () => void;
}

export interface Toast {
  id: number;
  tone: ToastTone;
  /** i18n key + params of the message. */
  messageKey: string;
  params?: Record<string, string>;
  /** Optional backend-provided i18n key shown under the message. */
  detailKey?: string;
  action?: ToastAction;
  /** Identical `dedupeKey`s replace each other instead of stacking. */
  dedupeKey?: string;
}

export const TOAST_LIMIT = 3;
export const TOAST_TTL_MS = 8000;

interface ToastState {
  toasts: Toast[];
  nextId: number;
  push: (toast: Omit<Toast, "id">) => number;
  dismiss: (id: number) => void;
  reset: () => void;
}

export const useToastStore = create<ToastState>()((set, get) => ({
  toasts: [],
  nextId: 1,
  push: (toast) => {
    const id = get().nextId;
    set((s) => {
      const rest = toast.dedupeKey ? s.toasts.filter((x) => x.dedupeKey !== toast.dedupeKey) : s.toasts;
      return { nextId: s.nextId + 1, toasts: [...rest, { ...toast, id }].slice(-TOAST_LIMIT) };
    });
    return id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
  reset: () => set({ toasts: [], nextId: 1 }),
}));
