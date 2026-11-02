// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "../../api/client";
import type { SocialAttachment, SocialChannel, SocialMessage } from "../../bindings";

const SEARCH_DEBOUNCE_MS = 250;

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
const { setMockSavePicker } = await import("../../api/dialog");
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

  it("filters conversations by name without searching players", async () => {
    let calls = 0;
    const search: Commands["socialSearchUsers"] = async () => {
      calls += 1;
      return { status: "ok", data: [] };
    };
    const channel = (
      id: string,
      name: string,
      kind: "global" | "direct",
    ): SocialChannel => ({
      id,
      kind,
      peer: kind === "direct" ? { ...SOCIAL_PEER, uuid: id, name } : null,
      lastMessage: null,
      unread: 0,
    });
    h.api = createMockCommands(
      {
        socialSearchUsers: search,
        socialChannels: async () => ({
          status: "ok",
          data: [
            channel("global", "Global", "global"),
            channel("dm:alex", "Alex", "direct"),
            channel("dm:steve", "Steve", "direct"),
          ],
        }),
      },
      { bus: createMockBus(), tickMs: 5 },
    );
    await connect();
    mount();
    const field = await screen.findByLabelText(i18n.t("social.filterChats"));
    fireEvent.change(field, { target: { value: "ale" } });
    await waitFor(() => expect(screen.queryByText("Steve")).toBeNull());
    expect(screen.getAllByText("Alex").length).toBeGreaterThan(0);
    await new Promise((resolve) => setTimeout(resolve, SEARCH_DEBOUNCE_MS + 50));
    // The chat list only filters channels; player lookup moved to the friends tab.
    expect(calls).toBe(0);
    expect(screen.queryByText(i18n.t("social.players"))).toBeNull();
  });

  it("sends on Enter and keeps Shift+Enter for a line break", async () => {
    await connect();
    const { container } = mount();
    // The mock global chat already holds other players' messages.
    await screen.findByLabelText(i18n.t("social.message"));
    const field = (await screen.findByLabelText(
      i18n.t("social.message"),
    )) as HTMLTextAreaElement;
    const ownBefore = container.querySelectorAll('[data-own="true"]').length;
    fireEvent.change(field, { target: { value: "hello" } });
    fireEvent.keyDown(field, { key: "Enter", shiftKey: true });
    expect(field.value).toBe("hello");
    expect(container.querySelectorAll('[data-own="true"]')).toHaveLength(ownBefore);
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
    // Two groups by the same author: two message headers plus the day split.
    expect(screen.getAllByText(SOCIAL_PEER.name)).toHaveLength(3);
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

  it("searches players from the friends tab and requests a friendship", async () => {
    await connect();
    const queries: string[] = [];
    const added: string[] = [];
    const search: Commands["socialSearchUsers"] = async (q) => {
      queries.push(q);
      return {
        status: "ok",
        data: [{ ...SOCIAL_PEER, uuid: SOCIAL_PEER.uuid, name: "Alex" }],
      };
    };
    const add: Commands["socialAddFriend"] = async (uuid) => {
      added.push(uuid);
      return { status: "ok", data: { user: SOCIAL_PEER, state: "outgoing" } };
    };
    h.api = createMockCommands(
      { socialSearchUsers: search, socialAddFriend: add },
      { bus: createMockBus(), tickMs: 5 },
    );
    mount();
    fireEvent.click(
      await screen.findByRole("button", { name: i18n.t("social.tabs.friends") }),
    );
    const field = await screen.findByLabelText(i18n.t("social.searchPlayers"));
    fireEvent.change(field, { target: { value: "ale" } });
    const result = await screen.findByTestId("player-search-result");
    expect(queries).toContain("ale");
    expect(within(result).getByText("Alex")).toBeTruthy();
    fireEvent.click(
      within(result).getByRole("button", { name: i18n.t("social.addFriend") }),
    );
    await waitFor(() => expect(added).toEqual([SOCIAL_PEER.uuid]));
  });

  it("has no write or friend buttons in a guest player profile", async () => {
    await loginGuest();
    mount();
    const field = await screen.findByLabelText(i18n.t("social.filterChats"));
    fireEvent.change(field, { target: { value: "ale" } });
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
  const shot: SocialMessage = {
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

  function mountImage(attachment?: Partial<SocialAttachment>) {
    const content = shot.content;
    const message: SocialMessage = {
      ...shot,
      content:
        content.type === "attachment" && content.attachment
          ? { ...content, attachment: { ...content.attachment, ...attachment } }
          : content,
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
  }

  it("opens a full-screen photo from the preview in the bubble", async () => {
    mountImage();
    await connect();
    mount();
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

  it("keeps the save button out of the bubble", async () => {
    mountImage();
    await connect();
    mount();
    await screen.findByRole("button", { name: i18n.t("social.openFullscreen") });
    expect(
      screen.queryByRole("button", { name: i18n.t("common.save") }),
    ).toBeNull();
  });

  /** Opens the viewer, saves and returns the path offered to the save dialog. */
  async function savedPath() {
    const paths: string[] = [];
    setMockSavePicker(async (options) => {
      paths.push(options?.defaultPath ?? "");
      return null;
    });
    await connect();
    mount();
    fireEvent.click(
      await screen.findByRole("button", { name: i18n.t("social.openFullscreen") }),
    );
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.save") }));
    await waitFor(() => expect(paths).toHaveLength(1));
    setMockSavePicker(null);
    return paths[0]!;
  }

  it("offers the original name with its own extension, never the stored webp", async () => {
    // The hub transcodes pictures to WebP but keeps the original name.
    mountImage({ fileName: "shot.PNG", mime: "image/webp" });
    expect(await savedPath()).toBe("shot.PNG");
    cleanup();
    mountImage({ fileName: "holiday.jpg", mime: "image/webp" });
    expect(await savedPath()).toBe("holiday.jpg");
  });

  it("falls back to .png when the original extension is unknown", async () => {
    mountImage({ fileName: "picture", mime: "image/webp" });
    expect(await savedPath()).toBe("picture.png");
    cleanup();
    mountImage({ id: "a9", fileName: "", mime: "image/jpeg" });
    expect(await savedPath()).toBe("a9.png");
  });
});

describe("read marker", () => {
  const history = (id: string): Commands["socialHistory"] => async () => ({
    status: "ok",
    data: {
      messages: [
        {
          id,
          channel: "global",
          author: SOCIAL_PEER,
          content: { type: "text", text: "hello" },
          createdAt: new Date().toISOString(),
          editedAt: null,
          deleted: false,
        },
      ],
      before: null,
    },
  });

  it("marks the channel read automatically and offers no manual button", async () => {
    const marked: Array<[string, string]> = [];
    h.api = createMockCommands(
      {
        socialHistory: history("7"),
        socialMarkRead: async (channel, messageId) => {
          marked.push([channel, messageId]);
          return { status: "ok", data: null };
        },
      },
      { bus: createMockBus(), tickMs: 5 },
    );
    await connect();
    mount();
    await screen.findByText("hello");
    await waitFor(() => expect(marked).toEqual([["global", "7"]]));
    // The old manual button (RU "Отметить прочитанным") is gone.
    expect(screen.queryByLabelText(/Отметить прочитанным|Mark as read/)).toBeNull();
  });
});

describe("friends", () => {
  it("offers write, profile and room invite actions per friend", async () => {
    await connect();
    mount();
    fireEvent.click(
      await screen.findByRole("button", { name: i18n.t("social.tabs.friends") }),
    );
    const write = await screen.findByRole("button", {
      name: i18n.t("social.write"),
    });
    const profile = screen.getByRole("button", {
      name: i18n.t("social.openProfile"),
    });
    const invite = screen.getByRole("button", {
      name: i18n.t("social.invite"),
    }) as HTMLButtonElement;
    expect(write).toBeTruthy();
    expect(profile).toBeTruthy();
    // No active room in the mock: the invitation is disabled with a hint.
    expect(invite.disabled).toBe(true);
    expect(invite.closest("span")?.getAttribute("title")).toBe(
      i18n.t("social.inviteNoRoom"),
    );
  });
});
