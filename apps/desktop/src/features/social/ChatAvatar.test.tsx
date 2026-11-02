// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "../../api/client";
import type { SocialUser } from "../../bindings";

// A fresh, directly controllable mock backend per test (the app-wide `api` is a lazy proxy).
const h = vi.hoisted(() => ({ api: null as unknown as Commands }));
vi.mock("../../api/client", async (orig) => {
  const actual = await orig<typeof import("../../api/client")>();
  return {
    ...actual,
    api: new Proxy({} as Commands, {
      get: (_target, prop) =>
        (h.api as unknown as Record<string | symbol, unknown>)[prop],
    }),
  };
});

const { createMockCommands } = await import("../../mocks");
const { createMockBus } = await import("../../mocks/bus");
const { ChatAvatar } = await import("./ChatAvatar");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const PICTURE = "data:image/webp;base64,UklGRg==";

const ALEX: SocialUser = {
  uuid: "2".repeat(32),
  name: "Alex",
  skinUrl: null,
  status: "online",
  role: "user",
  isFriend: false,
  canMessage: true,
  canInvite: true,
  displayName: null,
  bio: null,
  accent: null,
};

/** Mounts the avatar on a shared query client, so re-renders reuse the cache. */
function mount(user: SocialUser | null, extra: Partial<Commands> = {}) {
  h.api = createMockCommands(extra, { bus: createMockBus(), tickMs: 5 });
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={qc}>
      <ChatAvatar user={user} />
    </QueryClientProvider>,
  );
  const rerender = (next: SocialUser | null) =>
    view.rerender(
      <QueryClientProvider client={qc}>
        <ChatAvatar user={next} />
      </QueryClientProvider>,
    );
  return { ...view, rerender };
}

const image = () => document.querySelector("img");

afterEach(() => {
  cleanup();
});

describe("chat avatar", () => {
  it("shows the picture the hub returned", async () => {
    const preview = vi.fn(async () => ok(PICTURE));
    mount({ ...ALEX, avatar: "hash-1" }, { socialAvatarPreview: preview });
    expect(screen.getByText("A")).toBeTruthy();
    await waitFor(() => expect(image()?.getAttribute("src")).toBe(PICTURE));
    expect(preview).toHaveBeenCalledWith(ALEX.uuid);
    expect(screen.queryByText("A")).toBeNull();
  });

  it("keeps the initials when the preview fails", async () => {
    const preview = vi.fn(async () => ({
      status: "error" as const,
      error: { code: "notFound" as const, message: "no avatar", details: null },
    }));
    mount({ ...ALEX, avatar: "hash-1" }, { socialAvatarPreview: preview });
    await waitFor(() => expect(preview).toHaveBeenCalled());
    expect(image()).toBeNull();
    expect(screen.getByText("A")).toBeTruthy();
  });

  it("ignores a payload that is not an inlined image", async () => {
    mount(
      { ...ALEX, avatar: "hash-1" },
      { socialAvatarPreview: async () => ok("https://hub.example/avatar.webp") },
    );
    await waitFor(() => expect(screen.getByText("A")).toBeTruthy());
    expect(image()).toBeNull();
  });

  it("does not ask the hub without an avatar id", async () => {
    const preview = vi.fn(async () => ok(PICTURE));
    mount(ALEX, { socialAvatarPreview: preview });
    expect(screen.getByText("A")).toBeTruthy();
    expect(image()).toBeNull();
    expect(preview).not.toHaveBeenCalled();
  });

  it("re-renders from the cache and refetches only when the avatar id changes", async () => {
    const preview = vi.fn(
      async (_uuid: string) => ok(PICTURE),
    );
    const { rerender } = mount(
      { ...ALEX, avatar: "hash-1" },
      { socialAvatarPreview: preview },
    );
    await waitFor(() => expect(image()?.getAttribute("src")).toBe(PICTURE));
    expect(preview).toHaveBeenCalledTimes(1);
    rerender({ ...ALEX, avatar: "hash-1" });
    rerender({ ...ALEX, avatar: "hash-1" });
    expect(preview).toHaveBeenCalledTimes(1);
    rerender({ ...ALEX, avatar: "hash-2" });
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(2));
  });
});
