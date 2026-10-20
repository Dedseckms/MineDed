// The only entry point from UI code to the backend.
// Real Tauri bindings inside the app; mocks in a plain browser / tests
// (or when VITE_USE_MOCKS=1).
import { commands } from "../bindings";
import type { AppError } from "../bindings";
import { createMockCommands } from "../mocks";

export type Commands = typeof commands;

type Res<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const isMock: boolean = import.meta.env.VITE_USE_MOCKS === "1" || !inTauri;

// A little latency in the browser makes loading states visible; none in tests.
const mockLatency = import.meta.env.MODE === "test" ? 0 : 120;

export const api: Commands = isMock ? createMockCommands({}, { latencyMs: mockLatency }) : commands;

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
