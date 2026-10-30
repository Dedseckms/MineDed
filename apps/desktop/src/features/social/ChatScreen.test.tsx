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
const { MOCK_ONLINE, SOCIAL_PEER } = await import("../../mocks/social");
const { unwrap } = await import("../../api/client");
const { useSocialStore } = await import("../../stores/socialStore");
const { ChatScreen } = await import("./ChatScreen");
const i18n = (await import("../../i18n")).default;

// happy-dom has no Web Animations: motion falls back to instant presets.
for (const proto of [Element.prototype, HTMLElement.prototype])
  Reflect.deleteProperty(proto, "animate");

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
  it("hides administration from regular players and moderators", async () => {
    useSocialStore.getState().set({ self: SOCIAL_PEER, connected: true });
    mount();
    expect(
      screen.queryByRole("button", { name: i18n.t("social.tabs.admin") }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: i18n.t("social.tabs.privacy") }),
    ).toBeTruthy();
    cleanup();
    useSocialStore
      .getState()
      .set({ self: { ...SOCIAL_PEER, role: "moderator" }, connected: true });
    mount();
    expect(
      screen.queryByRole("button", { name: i18n.t("social.tabs.admin") }),
    ).toBeNull();
  });
  it("opens the admin panel for administrators", async () => {
    useSocialStore
      .getState()
      .set({ self: { ...SOCIAL_PEER, role: "admin" }, connected: true });
    mount();
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.tabs.admin") }),
    );
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.admin.title"))).toBeTruthy(),
    );
    expect(
      screen.getByRole("tab", { name: i18n.t("social.admin.tabs.journal") }),
    ).toBeTruthy();
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

describe("launcher presence", () => {
  it("shows how many launchers are connected", async () => {
    await connect();
    mount();
    expect(
      await screen.findByTitle(
        i18n.t("social.online", { n: MOCK_ONLINE }),
      ),
    ).toBeTruthy();
  });
});

describe("guest access", () => {
  /** Connects as a server account, then switches the session to its guest view. */
  async function loginGuest() {
    const user = await connect();
    useSocialStore
      .getState()
      .set({ self: { ...user, role: "guest", guest: true } });
  }

  it("marks the badge and keeps the public chat read-only", async () => {
    await loginGuest();
    mount();
    expect(await screen.findByText(i18n.t("social.guestBadge"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.guestReadOnly"))).toBeTruthy();
    expect(screen.queryByLabelText(i18n.t("social.message"))).toBeNull();
    expect(screen.queryByText(i18n.t("social.sections.direct"))).toBeNull();
    expect(
      screen.queryByRole("button", { name: i18n.t("social.tabs.friends") }),
    ).toBeNull();
  });

  it("has no write or friend buttons in a player profile", async () => {
    await loginGuest();
    mount();
    const search = await screen.findByLabelText(i18n.t("social.search"));
    fireEvent.change(search, { target: { value: "ale" } });
    await waitFor(() =>
      expect(screen.getAllByText(SOCIAL_PEER.name).length).toBeGreaterThan(0),
    );
    fireEvent.click(screen.getAllByText(SOCIAL_PEER.name)[0]!);
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.profile.title"))).toBeTruthy(),
    );
    expect(
      screen.queryByRole("button", { name: i18n.t("social.write") }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: i18n.t("social.addFriend") }),
    ).toBeNull();
  });

  it("logs in with a validated nickname from the account screen", async () => {
    mount();
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.guestLogin") }),
    );
    const field = await screen.findByLabelText(i18n.t("social.guestName"));
    fireEvent.change(field, { target: { value: "ab" } });
    expect(await screen.findByText(i18n.t("validation.nick.length"))).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: i18n.t("social.guestSubmit"),
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    fireEvent.change(field, { target: { value: "Steve" } });
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.guestSubmit") }),
    );
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.guestReadOnly"))).toBeTruthy(),
    );
    expect(useSocialStore.getState().self?.guest).toBe(true);
  });
});

describe("photo viewer", () => {
  it("opens a full-screen photo from an image bubble", async () => {
    const message: SocialMessage = {
      id: "1",
      channel: "global",
      author: SOCIAL_PEER,
      content: {
        type: "attachment",
        caption: null,
        attachment: {
          id: "a1",
          fileName: "shot.png",
          mime: "image/png",
          size: 2048,
          url: "",
          thumbUrl: null,
          width: null,
          height: null,
        },
      },
      createdAt: new Date().toISOString(),
      editedAt: null,
      deleted: false,
    };
    h.api = createMockCommands(
      {
        socialHistory: async () => ({
          status: "ok",
          data: { messages: [message], before: null },
        }),
        socialAttachmentPreview: async () => ({
          status: "ok",
          data: "data:image/webp;base64,AAAA",
        }),
      },
      { bus: createMockBus(), tickMs: 5 },
    );
    await connect();
    mount();
    fireEvent.click(
      await screen.findByRole("button", {
        name: i18n.t("social.showPreview"),
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: i18n.t("social.openFullscreen"),
      }),
    );
    expect(
      screen.getByRole("dialog", { name: i18n.t("social.photoViewer") }),
    ).toBeTruthy();
    expect(screen.getByRole("img", { name: "shot.png" })).toBeTruthy();
    expect(
      screen.getByRole("button", { name: i18n.t("common.save") }),
    ).toBeTruthy();
  });
});
