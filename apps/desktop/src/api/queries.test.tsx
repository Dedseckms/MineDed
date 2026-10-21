// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useTaskStore } from "../stores/taskStore";
import { api, unwrap } from "./client";
import { qk, useAddOfflineAccount, useSaveSettings, useSettings, useStartTask } from "./queries";

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { qc, Wrapper };
}

describe("useStartTask", () => {
  beforeEach(() => {
    useTaskStore.getState().reset();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => vi.useRealTimers());

  it("feeds channel events into the task store and invalidates on finish", async () => {
    const { qc, Wrapper } = wrapper();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    const onFinished = vi.fn();
    const { result } = renderHook(
      () =>
        useStartTask<string>({
          run: (name, ch) =>
            api.instanceCreate(
              { name, mc: "1.20.1", loader: { kind: "vanilla", version: null }, preset: null, icon: null },
              ch,
            ),
          invalidate: [qk.instances],
          onFinished,
        }),
      { wrapper: Wrapper },
    );

    let taskId = "";
    await act(async () => {
      taskId = await result.current.mutateAsync("Hook test");
    });
    expect(useTaskStore.getState().tasks[taskId]?.status).toBe("running");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(useTaskStore.getState().tasks[taskId]?.status).toBe("succeeded");
    expect(onFinished).toHaveBeenCalledWith(null, "Hook test");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.instances });
    expect(unwrap(await api.instanceList()).some((i) => i.name === "Hook test")).toBe(true);
  });

  it("surfaces command errors through the mutation", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(
      () =>
        useStartTask<null>({
          run: (_v, ch) =>
            api.instanceCreate(
              { name: "x", mc: "1.21", loader: { kind: "fabric", version: null }, preset: null, icon: null },
              ch,
            ),
        }),
      { wrapper: Wrapper },
    );
    await act(async () => {
      await result.current.mutateAsync(null).catch(() => {});
    });
    await waitFor(() => expect(result.current.error).toBeTruthy());
  });
});

describe("settings & accounts hooks", () => {
  it("saves settings optimistically", async () => {
    const { qc, Wrapper } = wrapper();
    const { result } = renderHook(() => ({ s: useSettings(), save: useSaveSettings() }), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.s.data).toBeDefined());
    const current = result.current.s.data;
    if (!current) throw new Error("no settings");
    await act(async () => {
      await result.current.save.mutateAsync({ ...current, theme: "light" });
    });
    expect(qc.getQueryData<typeof current>(qk.settings)?.theme).toBe("light");
    expect(unwrap(await api.settingsGet()).theme).toBe("light");
  });

  it("adds an offline account", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => useAddOfflineAccount(), { wrapper: Wrapper });
    await act(async () => {
      await result.current.mutateAsync("Hook_User");
    });
    expect(unwrap(await api.accountList()).some((a) => a.name === "Hook_User")).toBe(true);
  });
});
