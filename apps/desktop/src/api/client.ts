// The only entry point from UI code to the backend.
// Real Tauri bindings inside the app; mocks in a plain browser / tests
// (or when VITE_USE_MOCKS=1). Mocks are loaded with a dynamic `import()` and
// only in dev/test/explicit-mock builds, so production bundles never contain them.
import { commands } from "../bindings";
import type { AppError } from "../bindings";

export type Commands = typeof commands;

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Statically `false` in a production build without `VITE_USE_MOCKS=1` → mock code is tree-shaken. */
const mocksAllowed: boolean =
  import.meta.env.DEV || import.meta.env.MODE === "test" || import.meta.env.VITE_USE_MOCKS === "1";

export const isMock: boolean = mocksAllowed && (import.meta.env.VITE_USE_MOCKS === "1" || !inTauri);

/** Mock showcase mode for screenshots (`?demo=1`): hides mock markers, seeds a lived-in state. */
export const isDemo: boolean =
  isMock && typeof window !== "undefined" && new URLSearchParams(window.location.search).has("demo");

/** Version shown in demo mode (the mock one looks like a placeholder). */
const DEMO_VERSION = "1.0.5";

type MocksModule = typeof import("../mocks");

let mocksPromise: Promise<MocksModule> | null = null;

/** Lazily loads the mock backend module (mock mode only). */
export function loadMocks(): Promise<MocksModule> {
  if (!mocksAllowed) return Promise.reject(new Error("mocks are not available in this build"));
  mocksPromise ??= import("../mocks");
  return mocksPromise;
}

function createLazyMockApi(): Commands {
  // A little latency in the browser makes loading states visible; none in tests.
  const latencyMs = import.meta.env.MODE === "test" ? 0 : 120;
  let impl: Promise<Commands> | null = null;
  const get = () =>
    (impl ??= loadMocks().then((m) =>
      m.createMockCommands(isDemo ? m.demoOverrides(DEMO_VERSION) : {}, { latencyMs }),
    ));
  // Every command returns a Promise, so each one can await the module first.
  return new Proxy({} as Commands, {
    get(_target, prop: string | symbol) {
      if (typeof prop === "symbol" || prop === "then") return undefined;
      return async (...args: unknown[]) => {
        const target = (await get())[prop as keyof Commands] as (...a: unknown[]) => unknown;
        return target(...args);
      };
    },
  });
}

export const api: Commands = isMock ? createLazyMockApi() : commands;

export class ApiError extends Error {
  constructor(public readonly error: AppError) {
    super(`${error.code}: ${error.message}`);
  }
}

/** Turns a tauri-specta `Result` into a value or a thrown `ApiError` (for TanStack Query). */
export function unwrap<T>(res: Res<T>): T {
  if (res.status === "error") throw new ApiError(res.error);
  return res.data;
}
