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
const { unwrap } = await import("../../api/client");
const { useSocialStore } = await import("../../stores/socialStore");
const { useToastStore } = await import("../../stores/toastStore");
const { useNavStore } = await import("../../stores/navStore");
const { ChatScreen } = await import("./ChatScreen");
const i18n = (await import("../../i18n")).default;

for (const proto of [Element.prototype, HTMLElement.prototype])
  Reflect.deleteProperty(proto, "animate");

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

async function connect() {
  const user = unwrap(await h.api.socialConnect("acc-mineded"));
  act(() => useSocialStore.getState().set({ self: user, connected: true }));
  return user;
}

async function loginGuest() {
  const user = await connect();
  act(() =>
    useSocialStore
      .getState()
      .set({ self: { ...user, role: "guest", guest: true } }),
  );
  return user;
}

const ask = () =>
  screen.queryByRole("button", { name: i18n.t("social.accountRequest.ask") });

/** Records the raw arguments of every account request (the final contract has three). */
function recordRequests(calls: unknown[][]) {
  return {
    socialRequestAccount: (async (...args: unknown[]) => {
      calls.push(args);
      return { status: "ok" as const, data: null };
    }) as unknown as Commands["socialRequestAccount"],
  };
}

async function fillCredentials(login: string, password: string) {
  fireEvent.change(await screen.findByLabelText(i18n.t("social.accountRequest.login")), {
    target: { value: login },
  });
  fireEvent.change(screen.getByLabelText(i18n.t("social.accountRequest.password")), {
    target: { value: password },
  });
}

beforeEach(() => {
  vi.stubGlobal("matchMedia", media);
  localStorage.clear();
  useSocialStore.getState().reset();
  useToastStore.getState().reset();
  h.api = createMockCommands({}, { bus: createMockBus(), tickMs: 5 });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("guest account request", () => {
  it("shows the ask button to guests only", async () => {
    await connect();
    mount();
    await screen.findByLabelText(i18n.t("social.message"));
    expect(ask()).toBeNull();
    cleanup();

    await loginGuest();
    mount();
    expect(await screen.findByRole("button", { name: i18n.t("social.accountRequest.ask") })).toBeTruthy();
  });

  it("sends the login and password, shows a toast and stays sent afterwards", async () => {
    const calls: unknown[][] = [];
    h.api = createMockCommands(recordRequests(calls), { bus: createMockBus(), tickMs: 5 });
    const guest = await loginGuest();
    mount();
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("social.accountRequest.ask") }));
    await fillCredentials("Steve_01", "secret123");
    const password = screen.getByLabelText(
      i18n.t("social.accountRequest.password"),
    ) as HTMLInputElement;
    expect(password.type).toBe("password");
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.showPassword") }),
    );
    expect(password.type).toBe("text");
    fireEvent.change(
      screen.getByLabelText(i18n.t("social.accountRequest.note")),
      { target: { value: "  хочу свой ник  " } },
    );
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.submit") }),
    );
    await waitFor(() =>
      expect(calls).toEqual([["Steve_01", "secret123", "хочу свой ник"]]),
    );
    expect(
      useToastStore.getState().toasts.map((x) => x.messageKey),
    ).toContain("social.accountRequest.sentToast");
    expect(localStorage.getItem(`mineded.accountRequest.${guest.uuid}`)).toBe("1");
    const sent = await screen.findByRole("button", {
      name: i18n.t("social.accountRequest.sent"),
    });
    expect((sent as HTMLButtonElement).disabled).toBe(true);
    cleanup();

    // After a restart the state comes from localStorage: no duplicate request.
    mount();
    const again = await screen.findByRole("button", {
      name: i18n.t("social.accountRequest.sent"),
    });
    expect((again as HTMLButtonElement).disabled).toBe(true);
    expect(calls).toHaveLength(1);
  });

  it("validates the login and the password before sending", async () => {
    const calls: unknown[][] = [];
    h.api = createMockCommands(recordRequests(calls), { bus: createMockBus(), tickMs: 5 });
    await loginGuest();
    mount();
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("social.accountRequest.ask") }));
    await fillCredentials("ab!", "123");
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.submit") }),
    );
    expect(await screen.findByText(i18n.t("validation.nick.chars"))).toBeTruthy();
    expect(
      screen.getByText(i18n.t("validation.password.range")),
    ).toBeTruthy();
    expect(calls).toHaveLength(0);
  });

  it("keeps the button active when the backend refuses", async () => {
    h.api = createMockCommands(
      {
        socialRequestAccount: async () => ({
          status: "error",
          error: { code: "network", message: "offline", details: null },
        }),
      },
      { bus: createMockBus(), tickMs: 5 },
    );
    const guest = await loginGuest();
    mount();
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("social.accountRequest.ask") }));
    await fillCredentials("Steve", "secret123");
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.submit") }),
    );
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain(
        i18n.t("errors.network"),
      ),
    );
    expect(localStorage.getItem(`mineded.accountRequest.${guest.uuid}`)).toBeNull();
    expect(ask()).toBeTruthy();
  });

  it("replaces the ask button with a banner once the account exists", async () => {
    const uuid = await loginGuest().then((u) => u.uuid);
    const profile = unwrap(
      await createMockCommands({}, { bus: createMockBus(), tickMs: 5 }).socialConnect("acc-mineded"),
    );
    Object.assign(h.api, {
      socialProfile: async () =>
        ({
          status: "ok",
          data: { ...profile, uuid, role: "guest", guest: true, accountCreated: true },
        }) as never,
    });
    mount();
    expect(
      await screen.findByText(i18n.t("social.accountRequest.createdBanner")),
    ).toBeTruthy();
    expect(ask()).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.createdAction") }),
    );
    expect(useNavStore.getState().section).toBe("accounts");
  });
});

describe("note limit", () => {
  it("caps the note at 300 characters before sending", async () => {
    const calls: unknown[][] = [];
    h.api = createMockCommands(recordRequests(calls), { bus: createMockBus(), tickMs: 5 });
    await loginGuest();
    mount();
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("social.accountRequest.ask") }));
    const field = (await screen.findByLabelText(
      i18n.t("social.accountRequest.note"),
    )) as HTMLTextAreaElement;
    expect(field.maxLength).toBe(300);
    expect(
      screen.getByText(
        i18n.t("social.accountRequest.noteHint", { max: 300 }),
      ),
    ).toBeTruthy();
    await fillCredentials("Steve", "secret123");
    fireEvent.change(field, { target: { value: "я".repeat(400) } });
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.accountRequest.submit") }),
    );
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toHaveLength(3);
    expect(calls[0]![2]).toHaveLength(300);
  });
});
