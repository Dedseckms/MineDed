// Mock self-updater: `update_check` / `update_install`. Owned by ui.
import type { AppError, UpdateInfo, commands } from "../bindings";
import type { MockBus } from "./bus";

type Commands = typeof commands;
type Channel = Parameters<Commands["updateInstall"]>[0];

export const MOCK_CURRENT_VERSION = "0.0.0-mock";

export const MOCK_UPDATE: UpdateInfo = {
  version: "0.0.1-mock",
  currentVersion: MOCK_CURRENT_VERSION,
  notes: "• Faster instance launch\n• Fixed room chat scrolling\n• Updated translations",
  date: "2026-10-24T10:00:00Z",
};

/**
 * What the next `update_check` returns: an update, "up to date" (`null`) or
 * an error. In the browser an update is offered so the banner is visible;
 * tests start "up to date" and switch it explicitly.
 */
export const mockUpdateControl: { next: UpdateInfo | AppError | null; installError: AppError | null } = {
  next: import.meta.env.MODE === "test" ? null : MOCK_UPDATE,
  installError: null,
};

const isError = (v: UpdateInfo | AppError): v is AppError => "code" in v;

interface Ctx {
  bus: MockBus;
  wait: () => Promise<void>;
  runTask: (
    kind: "appUpdate",
    titleKey: string,
    ch: Channel,
    stages: Array<{ stage: string; total: number | null }>,
    onSuccess: () => AppError | null,
  ) => string;
}

export function createUpdateMocks(ctx: Ctx): Partial<Commands> {
  return {
    updateCheck: async () => {
      await ctx.wait();
      const next = mockUpdateControl.next;
      if (next && isError(next)) return { status: "error", error: next };
      if (next) ctx.bus.emit("updateAvailable", { ...next });
      return { status: "ok", data: next ? { ...next } : null };
    },

    updateInstall: async (ch) => {
      await ctx.wait();
      const next = mockUpdateControl.next;
      if (!next || isError(next))
        return { status: "error", error: { code: "notFound", message: "mock: no update available", details: null } };
      // Same bare stage names as the real updater (`check` → `download` → `install`).
      const id = ctx.runTask(
        "appUpdate",
        "tasks.appUpdate",
        ch,
        [
          { stage: "check", total: null },
          { stage: "download", total: 8_000_000 },
          { stage: "install", total: null },
        ],
        // The real app restarts here; the mock just stops offering the update.
        () => {
          if (mockUpdateControl.installError) return mockUpdateControl.installError;
          mockUpdateControl.next = null;
          return null;
        },
      );
      return { status: "ok", data: id };
    },
  };
}
