// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "./client";

// A fresh, directly controllable mock backend per test (the app-wide `api` is a lazy proxy).
const h = vi.hoisted(() => ({ api: null as unknown as Commands }));
vi.mock("./client", async (orig) => {
  const actual = await orig<typeof import("./client")>();
  return {
    ...actual,
    api: new Proxy({} as Commands, { get: (_t, p) => (h.api as unknown as Record<string | symbol, unknown>)[p] }),
  };
});

const { createMockBus } = await import("../mocks/bus");
const { createMockCommands } = await import("../mocks");
const { unwrap } = await import("./client");
const { qk, useLeaveRoom } = await import("./queries");

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const Wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  const { result } = renderHook(() => useLeaveRoom(), { wrapper: Wrapper });
  return { qc, result };
}

describe("useLeaveRoom", () => {
  beforeEach(() => {
    h.api = createMockCommands({}, { bus: createMockBus(), tickMs: 10 });
  });

  it("stops voice before leaving the room", async () => {
    unwrap(await h.api.roomJoin("VOXA-1111", null, null));
    unwrap(await h.api.voiceStart());
    expect(unwrap(await h.api.voiceStatus()).active).toBe(true);
    const { qc, result } = setup();
    qc.setQueryData(qk.voiceStatus, unwrap(await h.api.voiceStatus()));

    await act(async () => {
      await result.current.mutateAsync();
    });
    expect(unwrap(await h.api.roomState())).toBeNull();
    expect(unwrap(await h.api.voiceStatus()).active).toBe(false);
    expect(qc.getQueryState(qk.voiceStatus)?.isInvalidated).toBe(true);
  });

  it("still leaves when voice_stop fails or throws", async () => {
    const base = h.api;
    for (const voiceStop of [
      async () => ({ status: "error" as const, error: { code: "internal" as const, message: "x", details: null } }),
      async () => {
        throw new Error("ipc down");
      },
    ]) {
      h.api = createMockCommands({ voiceStop }, { bus: createMockBus(), tickMs: 10 });
      unwrap(await h.api.roomJoin("VOXA-2222", null, null));
      const { result } = setup();
      await act(async () => {
        await result.current.mutateAsync();
      });
      expect(unwrap(await h.api.roomState())).toBeNull();
    }
    h.api = base;
  });
});
