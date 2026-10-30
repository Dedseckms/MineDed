import { create } from "zustand";
import type { SocialReport } from "../bindings";

/**
 * Reports resolved during this session. The hub returns open reports only
 * (`GET /chat/mod/reports`), so the "resolved" filter shows what was closed
 * since the launcher started.
 */
interface AdminState {
  resolved: SocialReport[];
  addResolved: (report: SocialReport) => void;
  reset: () => void;
}

export const useAdminStore = create<AdminState>()((set) => ({
  resolved: [],
  addResolved: (report) =>
    set((s) =>
      s.resolved.some((r) => r.id === report.id)
        ? s
        : { resolved: [report, ...s.resolved] },
    ),
  reset: () => set({ resolved: [] }),
}));
