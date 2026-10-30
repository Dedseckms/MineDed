// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "../../api/client";
import type { SocialMessage } from "../../bindings";

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

const { createMockBus } = await import("../../mocks/bus");
const { createMockCommands } = await import("../../mocks");
const { SOCIAL_PEER } = await import("../../mocks/social");
const { unwrap } = await import("../../api/client");
const { useSocialStore } = await import("../../stores/socialStore");
const { ChatScreen } = await import("./ChatScreen");
const i18n = (await import("../../i18n")).default;

/** Desktop window: the chat list and the conversation are both visible. */
function media(query: string) {
  return {
    matches: query.includes("min-width"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  } as unknown as MediaQueryList;
}

function mount() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

/** Own-server account: the mock only connects Yggdrasil sessions to chat. */
async function connect() {
  const user = unwrap(await h.api.socialConnect("acc-mineded"));
  act(() => useSocialStore.getState().set({ self: user, connected: true }));
  return user;
}

beforeEach(() => {
  vi.stubGlobal("matchMedia", media);
  useSocialStore.getState().reset();
  h.api = createMockCommands({}, { bus: createMockBus(), tickMs: 5 });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("chat access", () => {
  it("explains the account requirement before showing conversations", () => {
    mount();
    expect(screen.getByText(i18n.t("social.login"))).toBeTruthy();
    expect(screen.queryByLabelText(i18n.t("social.message"))).toBeNull();
  });
  it("hides administration from regular players", async () => {
    useSocialStore.getState().set({ self: SOCIAL_PEER, connected: true });
    mount();
    expect(
      screen.queryByRole("button", { name: i18n.t("social.tabs.admin") }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: i18n.t("social.tabs.privacy") }),
    ).toBeTruthy();
  });
  it("shows moderation for moderators", async () => {
    useSocialStore
      .getState()
      .set({ self: { ...SOCIAL_PEER, role: "moderator" }, connected: true });
    mount();
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.tabs.admin") }),
    );
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.reports"))).toBeTruthy(),
    );
    expect(screen.queryByText(i18n.t("social.server"))).toBeNull();
  });
});

describe("telegram layout", () => {
  it("lists the public chat and direct chats in sections", async () => {
    await connect();
    mount();
    expect(
      (await screen.findAllByText(i18n.t("social.sections.global"))).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText(i18n.t("social.sections.direct"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.noChats"))).toBeTruthy();
  });

  it("finds players from the list search", async () => {
    await connect();
    mount();
    const field = await screen.findByLabelText(i18n.t("social.search"));
    fireEvent.change(field, { target: { value: "ale" } });
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.players"))).toBeTruthy(),
    );
    await waitFor(() =>
      expect(screen.getAllByText(SOCIAL_PEER.name).length).toBeGreaterThan(0),
    );
  });

  it("sends on Enter and keeps Shift+Enter for a line break", async () => {
    await connect();
    const { container } = mount();
    await screen.findByText(i18n.t("social.empty"));
    const field = (await screen.findByLabelText(
      i18n.t("social.message"),
    )) as HTMLTextAreaElement;
    fireEvent.change(field, { target: { value: "hello" } });
    fireEvent.keyDown(field, { key: "Enter", shiftKey: true });
    expect(field.value).toBe("hello");
    expect(container.querySelectorAll('[data-own="true"]')).toHaveLength(0);
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() =>
      expect(container.querySelectorAll('[data-own="true"]')).toHaveLength(1),
    );
    expect(field.value).toBe("");
  });

  it("groups runs, marks own bubbles and splits days", async () => {
    const now = new Date();
    const at = (days: number, hour: number, minute = 0) => {
      const d = new Date(now);
      d.setDate(d.getDate() - days);
      d.setHours(hour, minute, 0, 0);
      return d.toISOString();
    };
    const self = unwrap(await h.api.socialConnect("acc-mineded"));
    const message = (
      id: string,
      at: string,
      body: string,
      mine = false,
    ): SocialMessage => ({
      id,
      channel: "global",
      author: mine ? self : SOCIAL_PEER,
      content: { type: "text", text: body },
      createdAt: at,
      editedAt: null,
      deleted: false,
    });
    const messages = [
      message("1", at(0, 10, 0), "first"),
      message("2", at(0, 10, 2), "second"),
      message("3", at(0, 10, 3), "mine", true),
      message("4", at(1, 9, 0), "old"),
    ];
    const history: Commands["socialHistory"] = async () => ({
      status: "ok",
      data: { messages, before: null },
    });
    h.api = createMockCommands({ socialHistory: history }, {
      bus: createMockBus(),
      tickMs: 5,
    });
    await connect();
    const { container } = mount();
    await waitFor(() => expect(screen.getByText("first")).toBeTruthy());
    expect(screen.getByText("second")).toBeTruthy();
    expect(screen.getByText(i18n.t("social.date.today"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.date.yesterday"))).toBeTruthy();
    expect(container.querySelectorAll('[data-own="true"]')).toHaveLength(1);
    expect(container.querySelectorAll('[data-own="false"]')).toHaveLength(3);
    // Two groups by the same author: the name appears once per group.
    expect(screen.getAllByText(SOCIAL_PEER.name)).toHaveLength(2);
  });
});
