// Mock backend for UI work without Rust. Owned by ui.
// Every command not listed in `overrides` resolves to a `notImplemented` error,
// so the mock always satisfies the full generated `commands` type.
import type { commands } from "../bindings";

type Commands = typeof commands;

const notImplemented = (name: string) => async () => ({
  status: "error" as const,
  error: { code: "notImplemented" as const, message: `mock: ${name}`, details: null },
});

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const overrides: Partial<Commands> = {
  appVersion: async () => "0.0.0-mock",
  instanceList: async () => ok([]),
  accountList: async () => ok([]),
  tasksList: async () => ok([]),
};

export function createMockCommands(extra: Partial<Commands> = {}): Commands {
  const impl: Partial<Commands> = { ...overrides, ...extra };
  return new Proxy(impl, {
    get(target, prop: string) {
      return target[prop as keyof Commands] ?? notImplemented(prop);
    },
  }) as Commands;
}
