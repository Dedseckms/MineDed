// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { InstanceSpec } from "../bindings";
import { useTaskStore } from "../stores/taskStore";
import { useToastStore } from "../stores/toastStore";
import type { Commands } from "./client";

// Wraps the mock backend so `presetApply` can be observed / overridden.
const h = vi.hoisted(() => ({
  presetApply: null as null | ((...a: unknown[]) => unknown),
  calls: [] as unknown[][],
}));

vi.mock("./client", async (orig) => {
  const actual = await orig<typeof import("./client")>();
  const real = actual.api as unknown as Record<string, (...a: unknown[]) => unknown>;
  return {
    ...actual,
    api: new Proxy({} as Commands, {
      get: (_t, p: string) => {
        if (p !== "presetApply") return real[p];
        return (...args: unknown[]) => {
          h.calls.push(args);
          return (h.presetApply ?? real.presetApply)?.(...args);
        };
      },
    }),
  };
});

const { api, unwrap } = await import("./client");
const { qk, useCreateInstance, usePresetApply } = await import("./queries");

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { qc, Wrapper };
}

const spec = (name: string): InstanceSpec => ({
  name,
  mc: "1.20.1",
  loader: { kind: "vanilla", version: null },
  preset: null,
  icon: null,
});

const settle = () =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(20_000);
  });

describe("preset apply chaining", () => {
  beforeEach(() => {
    useTaskStore.getState().reset();
    useToastStore.getState().reset();
    h.presetApply = null;
    h.calls = [];
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => vi.useRealTimers());

  it("starts preset_apply on the new instance after create succeeds", async () => {
    const { qc, Wrapper } = wrapper();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    const { result } = renderHook(() => useCreateInstance(), { wrapper: Wrapper });

    await act(async () => {
      await result.current.create({ spec: spec("Preset chain"), preset: "potato" });
    });
    await settle();

    const created = unwrap(await api.instanceList()).find((i) => i.name === "Preset chain");
    expect(created).toBeDefined();
    expect(h.calls).toHaveLength(1);
    expect(h.calls[0]?.slice(0, 2)).toEqual([created?.id, "potato"]);
    expect(created?.preset).toBe("potato");

    const tasks = Object.values(useTaskStore.getState().tasks).map((t) => [t.kind, t.status]);
    expect(tasks).toEqual(
      expect.arrayContaining([
        ["instanceCreate", "succeeded"],
        ["presetApply", "succeeded"],
      ]),
    );
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.content(created?.id ?? "") });
    const toast = useToastStore.getState().toasts.at(-1);
    expect(toast).toMatchObject({ tone: "info", messageKey: "preset.applied", detailKey: "preset.appliedHint" });
    expect(toast?.action?.labelKey).toBe("preset.openInstance");
  });

  it("does not apply anything without a preset", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => useCreateInstance(), { wrapper: Wrapper });
    await act(async () => {
      await result.current.create({ spec: spec("No preset"), preset: null });
    });
    await settle();
    expect(h.calls).toHaveLength(0);
    expect(unwrap(await api.instanceList()).find((i) => i.name === "No preset")?.preset).toBeNull();
  });

  it("toasts an error when the chained apply cannot start", async () => {
    h.presetApply = () =>
      Promise.resolve({ status: "error", error: { code: "notFound", message: "gone", details: null } });
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => useCreateInstance(), { wrapper: Wrapper });
    await act(async () => {
      await result.current.create({ spec: spec("Apply fails"), preset: "quality" });
    });
    await settle();
    expect(h.calls).toHaveLength(1);
    expect(useToastStore.getState().toasts.at(-1)).toMatchObject({
      tone: "danger",
      messageKey: "preset.applyFailed",
      params: { name: "Apply fails" },
      error: { code: "notFound" },
    });
  });

  it("usePresetApply re-reads the instance and toasts without an open action", async () => {
    const { qc, Wrapper } = wrapper();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    const { result } = renderHook(() => usePresetApply(), { wrapper: Wrapper });
    await act(async () => {
      await result.current.mutateAsync({ id: "inst-classic", preset: "balanced" });
    });
    await settle();
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.instances });
    expect(unwrap(await api.instanceGet("inst-classic")).preset).toBe("balanced");
    const toast = useToastStore.getState().toasts.at(-1);
    expect(toast).toMatchObject({ messageKey: "preset.applied" });
    expect(toast?.action).toBeUndefined();
  });

  it("does not toast success when the apply task fails", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => usePresetApply(), { wrapper: Wrapper });
    await act(async () => {
      await result.current.mutateAsync({ id: "missing-instance", preset: "balanced" }).catch(() => {});
    });
    await settle();
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});
