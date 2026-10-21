// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { InstalledContent } from "../bindings";
import { useDebounced } from "../hooks/useDebounced";
import { api, unwrap } from "./client";
import {
  qk,
  useAddSkin,
  useApplySkin,
  useContentList,
  useContentSearch,
  useLoaderVersions,
  useResolveContent,
  useSetContentEnabled,
  useSkins,
} from "./queries";

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const Wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  return { qc, Wrapper };
}

describe("content hooks", () => {
  it("useContentSearch loads pages until the end", async () => {
    const { Wrapper } = wrapper();
    const q = {
      source: "modrinth" as const,
      kind: "mod" as const,
      text: "",
      mc: "1.21.1",
      loader: "fabric" as const,
      categories: [],
      sort: "downloads" as const,
      offset: 0,
      limit: 10,
    };
    const { result } = renderHook(() => useContentSearch(q), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(1));
    expect(result.current.hasNextPage).toBe(true);
    await act(async () => {
      await result.current.fetchNextPage();
    });
    const total = result.current.data?.pages[0]?.total ?? 0;
    await waitFor(() => expect(result.current.data?.pages).toHaveLength(2));
    expect(result.current.data?.pages.flatMap((p) => p.items)).toHaveLength(Math.min(total, 20));
    expect(result.current.hasNextPage).toBe(total > 20);
  });

  it("useSetContentEnabled updates the list optimistically", async () => {
    const { qc, Wrapper } = wrapper();
    const { result } = renderHook(() => ({ list: useContentList("inst-survival"), toggle: useSetContentEnabled("inst-survival") }), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(result.current.list.data).toBeDefined());
    const first = result.current.list.data?.[0];
    if (!first) throw new Error("empty list");
    act(() => result.current.toggle.mutate({ fileName: first.fileName, enabled: !first.enabled }));
    await waitFor(() =>
      expect(qc.getQueryData<InstalledContent[]>(qk.content("inst-survival"))?.[0]?.enabled).toBe(!first.enabled),
    );
    await waitFor(() => expect(result.current.toggle.isSuccess).toBe(true));
    expect(unwrap(await api.contentList("inst-survival"))[0]?.enabled).toBe(!first.enabled);
  });

  it("useLoaderVersions is disabled for vanilla", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => ({ v: useLoaderVersions("vanilla", "1.21.1"), f: useLoaderVersions("quilt", "1.21.1") }), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(result.current.f.data?.length).toBeGreaterThan(0));
    expect(result.current.v.fetchStatus).toBe("idle");
  });

  it("useResolveContent returns a plan", async () => {
    const { Wrapper } = wrapper();
    const { result } = renderHook(() => useResolveContent(), { wrapper: Wrapper });
    const plan = await act(() =>
      result.current.mutateAsync({ instance: "inst-fabric", picks: [{ source: "modrinth", projectId: "Bh37bMuy", versionId: null }] }),
    );
    expect(plan.install.map((v) => v.projectId)).toEqual(["Bh37bMuy"]);
  });
});

describe("skin hooks", () => {
  it("adds a skin and applies it to an account", async () => {
    const { qc, Wrapper } = wrapper();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    const { result } = renderHook(() => ({ skins: useSkins(), add: useAddSkin(), apply: useApplySkin() }), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.skins.data).toBeDefined());
    const before = result.current.skins.data?.length ?? 0;
    const skin = await act(() => result.current.add.mutateAsync({ path: "C:\\x\\hero.png", name: "Hero", model: "classic" }));
    await waitFor(() => expect(result.current.skins.data).toHaveLength(before + 1));
    await act(() => result.current.apply.mutateAsync({ account: "acc-alex", skin: skin.id }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.accounts });
    expect(unwrap(await api.accountList()).find((a) => a.id === "acc-alex")?.skinUrl).toBe(skin.url);
  });
});

describe("useDebounced", () => {
  afterEach(() => vi.useRealTimers());
  it("delays value changes", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(({ v }) => useDebounced(v, 300), { initialProps: { v: "a" } });
    rerender({ v: "ab" });
    expect(result.current).toBe("a");
    act(() => {
      vi.advanceTimersByTime(299);
    });
    expect(result.current).toBe("a");
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("ab");
  });
});
