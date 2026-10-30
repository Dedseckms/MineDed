// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applySocialEvent, useSocialSync } from "./useSocialSync";
import { useSocialStore } from "../stores/socialStore";
import { useGameStore } from "../stores/gameStore";

const fixture = vi.hoisted(() => ({
  account: "first",
  room: false,
  order: [] as string[],
}));
const connect = vi.hoisted(() =>
  vi.fn(async (account: string) => {
    fixture.order.push(`connect:${account}`);
    return {
      status: "ok" as const,
      data: {
        uuid: account,
        name: account,
        skinUrl: null,
        status: "online" as const,
        role: "user" as const,
        isBlocked: false,
        isFriend: false,
        canMessage: true,
        canInvite: true,
      },
    };
  }),
);
const presence = vi.hoisted(() =>
  vi.fn(async () => ({ status: "ok" as const, data: null })),
);
vi.mock("../api/queries", () => ({
  useAccounts: () => ({
    data: [
      { id: fixture.account, isDefault: true, kind: "yggdrasil", valid: true },
    ],
  }),
  useRoomState: () => ({ data: fixture.room ? { roomId: "room" } : null }),
}));
vi.mock("../api/events", () => ({ subscribe: () => () => {} }));
vi.mock("../api/client", () => ({
  unwrap: (value: { data: unknown }) => value.data,
  api: {
    socialConnect: connect,
    socialDisconnect: async () => {
      fixture.order.push("disconnect");
      return { status: "ok", data: null };
    },
    socialChannels: async () => ({ status: "ok", data: [] }),
    socialSetPresence: presence,
  },
}));
function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
beforeEach(() => {
  fixture.account = "first";
  fixture.room = false;
  fixture.order = [];
  connect.mockClear();
  presence.mockClear();
  useSocialStore.getState().reset();
  useGameStore.getState().reset();
});
afterEach(cleanup);
describe("social connection synchronization", () => {
  it("changes account only after disconnect and clears private state", async () => {
    const view = renderHook(() => useSocialSync(), { wrapper });
    await waitFor(() =>
      expect(useSocialStore.getState().self?.uuid).toBe("first"),
    );
    act(() => {
      useSocialStore.getState().block("peer", true);
      fixture.account = "second";
    });
    view.rerender();
    await waitFor(() =>
      expect(useSocialStore.getState().self?.uuid).toBe("second"),
    );
    expect(useSocialStore.getState().blocked).toEqual([]);
    const second = fixture.order.indexOf("connect:second");
    expect(fixture.order[second - 1]).toBe("disconnect");
  });
  it("publishes actual game and room presence", async () => {
    const view = renderHook(() => useSocialSync(), { wrapper });
    await waitFor(() => expect(presence).toHaveBeenCalledWith("online"));
    act(() => useGameStore.getState().setState("instance", "running"));
    await waitFor(() => expect(presence).toHaveBeenCalledWith("inGame"));
    fixture.room = true;
    view.rerender();
    await waitFor(() => expect(presence).toHaveBeenCalledWith("inRoom"));
  });
});

it("deduplicates live messages without discarding older cursor pages", () => {
  const qc = new QueryClient();
  const author = {
    uuid: "peer",
    name: "Peer",
    skinUrl: null,
    status: "online" as const,
    role: "user" as const,
    isFriend: false,
    isBlocked: false,
    canMessage: true,
    canInvite: true,
  };
  const message = {
    id: "2",
    channel: "global",
    author,
    content: { type: "text" as const, text: "Hello" },
    createdAt: "2026-10-29T00:00:00Z",
    editedAt: null,
    deleted: false,
  };
  const oldPage = { messages: [], before: "old-cursor" };
  qc.setQueryData(["social", "history", "global"], {
    pages: [{ messages: [], before: "cursor" }, oldPage],
    pageParams: [null, "cursor"],
  });
  applySocialEvent(qc, { type: "message", message });
  applySocialEvent(qc, { type: "message", message });
  const data = qc.getQueryData<{
    pages: { messages: (typeof message)[]; before: string }[];
  }>(["social", "history", "global"]);
  expect(data?.pages[0]?.messages).toEqual([message]);
  expect(data?.pages[1]).toEqual(oldPage);
  applySocialEvent(qc, {
    type: "messageUpdated",
    message: { ...message, deleted: true },
  });
  expect(
    qc.getQueryData<{ pages: { messages: (typeof message)[] }[] }>([
      "social",
      "history",
      "global",
    ])?.pages[0]?.messages[0]?.deleted,
  ).toBe(true);
});
