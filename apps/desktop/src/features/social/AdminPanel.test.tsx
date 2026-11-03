// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "../../api/client";
import type {
  AccountRequest,
  SocialReport,
  SocialUser,
} from "../../bindings";
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
const { unwrap } = await import("../../api/client");
const { useAdminStore } = await import("../../stores/adminStore");
const { useToastStore } = await import("../../stores/toastStore");
const { AdminPanel } = await import("./admin/AdminPanel");
const i18n = (await import("../../i18n")).default;

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const AUTHOR = {
  uuid: "2".repeat(32),
  name: "Alex",
  skinUrl: null,
  status: "online" as const,
  role: "user" as const,
  isFriend: false,
  canMessage: true,
  canInvite: true,
};

const REPORT: SocialReport = {
  id: "report-1",
  message: {
    id: "m-1",
    channel: "global",
    author: AUTHOR,
    content: { type: "text", text: "buy cheap diamonds" },
    createdAt: new Date().toISOString(),
    editedAt: null,
    deleted: false,
  },
  reporter: { ...AUTHOR, uuid: "3".repeat(32), name: "Steve" },
  reason: "spam",
  createdAt: new Date().toISOString(),
};

const ADMIN: SocialUser = { ...AUTHOR, uuid: "1".repeat(32), name: "Admin", role: "admin" };

function mount(
  self: SocialUser = ADMIN,
  extra: Partial<Commands> = {},
) {
  Object.assign(h.api, extra);
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <AdminPanel self={self} />
    </QueryClientProvider>,
  );
}

beforeEach(async () => {
  useAdminStore.getState().reset();
  h.api = createMockCommands({}, {
    bus: createMockBus(),
    tickMs: 5,
    socialRole: "admin",
  });
  unwrap(await h.api.socialConnect("acc-mineded"));
});
afterEach(() => {
  cleanup();
});

const tab = (value: string) =>
  screen.getByRole("tab", { name: i18n.t(`social.admin.tabs.${value}`) });

describe("admin panel access", () => {
  it("is not rendered for players or moderators", () => {
    for (const role of ["user", "moderator", "guest"] as const) {
      const { container, unmount } = mount({ ...ADMIN, role });
      expect(container.firstChild).toBeNull();
      unmount();
    }
  });

  it("shows the full section to admins with all tabs", () => {
    mount();
    expect(screen.getByText(i18n.t("social.admin.title"))).toBeTruthy();
    for (const value of ["moderation", "users", "requests", "server", "journal"])
      expect(tab(value)).toBeTruthy();
  });
});

describe("moderation tab", () => {
  it("filters reports between open and resolved", async () => {
    let reports: SocialReport[] = [REPORT];
    mount(ADMIN, {
      socialReports: async () => ok(reports),
      socialResolveReport: async (id) => {
        reports = reports.filter((r) => r.id !== id);
        return ok(null);
      },
    });
    expect((await screen.findAllByText(AUTHOR.name)).length).toBeGreaterThan(0);
    expect(
      (screen.getAllByText("buy cheap diamonds").length),
    ).toBeGreaterThan(0);
    expect(screen.getByText("spam")).toBeTruthy();

    fireEvent.click(
      screen.getByRole("radio", {
        name: new RegExp(i18n.t("social.admin.filter.resolved")),
      }),
    );
    expect(screen.getByText(i18n.t("social.admin.noResolved"))).toBeTruthy();

    fireEvent.click(
      screen.getByRole("radio", {
        name: new RegExp(i18n.t("social.admin.filter.open")),
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: i18n.t("social.resolve") }),
    );
    await waitFor(() => expect(reports).toHaveLength(0));
    fireEvent.click(
      screen.getByRole("radio", {
        name: new RegExp(i18n.t("social.admin.filter.resolved")),
      }),
    );
    await waitFor(() =>
      expect(screen.getAllByText(AUTHOR.name).length).toBeGreaterThan(0),
    );
    expect(useAdminStore.getState().resolved).toHaveLength(1);
  });
});

describe("journal tab", () => {
  it("shows when, who, what and over whom", async () => {
    mount();
    fireEvent.click(tab("journal"));
    expect(
      await screen.findByText(i18n.t("social.admin.journal.when")),
    ).toBeTruthy();
    expect(screen.getByText(i18n.t("social.admin.journal.who"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.admin.journal.what"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.admin.journal.whom"))).toBeTruthy();
    expect((await screen.findAllByText("Admin")).length).toBeGreaterThan(0);
    expect(
      screen.getAllByText(i18n.t("social.admin.action.mute")).length,
    ).toBeGreaterThan(0);
  });
});

describe("server tab", () => {
  it("shows presence, disk and backup state", async () => {
    mount();
    fireEvent.click(tab("server"));
    expect(await screen.findByText(i18n.t("social.admin.online"))).toBeTruthy();
    expect(screen.getByText(i18n.t("social.admin.clients"))).toBeTruthy();
    expect(await screen.findByText("3", { selector: "p" })).toBeTruthy();
    expect(screen.getByText(/42\.0 \/ 100\.0/)).toBeTruthy();
    expect(
      screen.getByText(i18n.t("social.storage.newDisk")),
    ).toBeTruthy();
  });
});

describe("users tab", () => {
  it("lists every known account and filters it by the account search", async () => {
    mount();
    fireEvent.click(tab("users"));
    const filter = await screen.findByLabelText(
      i18n.t("social.admin.accounts.filter"),
    );
    // The mock knows a regular player and a guest.
    expect((await screen.findAllByText("Alex")).length).toBeGreaterThan(0);
    expect(screen.getByText("Gremlin_")).toBeTruthy();
    expect(screen.getAllByText(i18n.t("social.admin.accounts.real")).length).toBe(1);
    fireEvent.change(filter, { target: { value: "grem" } });
    await waitFor(() => expect(screen.queryByText("Alex")).toBeNull());
    expect(screen.getByText("Gremlin_")).toBeTruthy();
    fireEvent.change(filter, { target: { value: "zzz" } });
    expect(
      screen.getByText(i18n.t("social.admin.accounts.empty")),
    ).toBeTruthy();
  });

  it("shows an issued invite for the account", async () => {
    mount();
    fireEvent.click(tab("users"));
    fireEvent.click((await screen.findAllByText("Gremlin_"))[0]!);
    await waitFor(() =>
      expect(screen.getByText(i18n.t("social.admin.userUuid"))).toBeTruthy(),
    );
    expect(
      screen.queryByText(i18n.t("social.admin.accounts.invite")),
    ).toBeNull();

    const requests: AccountRequest[] = [
      {
        id: "9".repeat(32),
        name: "Invited",
        note: "",
        createdAt: new Date().toISOString(),
      },
    ];
    cleanup();
    mount(ADMIN, {
      socialAccountRequests: async () => ok(requests),
      socialAdminAccounts: async () =>
        ok([
          {
            uuid: requests[0]!.id,
            name: "Invited",
            guest: true,
            createdAt: requests[0]!.createdAt,
            invite: "MINE-DED1-NV1T",
            inviteUrl: "https://drasl.test/register?invite=MINE-DED1-NV1T",
          },
        ]),
    });
    fireEvent.click(tab("users"));
    fireEvent.click(await screen.findByText("Invited"));
    expect(
      await screen.findByText(i18n.t("social.admin.accounts.invite")),
    ).toBeTruthy();
    expect(
      screen.getByText("https://drasl.test/register?invite=MINE-DED1-NV1T"),
    ).toBeTruthy();
  });

  it("finds a player and shows role and sanctions", async () => {
    mount();
    fireEvent.click(tab("users"));
    fireEvent.change(screen.getByLabelText(i18n.t("social.search")), {
      target: { value: "ale" },
    });
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.search") }));
    fireEvent.click((await screen.findAllByText(AUTHOR.name))[0]!);
    expect(
      await screen.findByText(i18n.t("social.admin.userUuid")),
    ).toBeTruthy();
    expect(screen.getByText(i18n.t("social.admin.sanctions"))).toBeTruthy();
    expect(
      screen.getByText(i18n.t("social.admin.resetPending")),
    ).toBeTruthy();
    expect(
      (screen.getByRole("button", {
        name: i18n.t("social.admin.resetProfile"),
      }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });
});

describe("requests tab", () => {
  const REQUESTS: AccountRequest[] = [
    {
      id: "7".repeat(32),
      name: "NotchFan",
      note: "Хочу свой ник",
      createdAt: new Date().toISOString(),
    },
    {
      id: "8".repeat(32),
      name: "Kefir",
      note: "",
      createdAt: new Date().toISOString(),
    },
  ];

  it("creates the account of an approved request", async () => {
    const created: string[] = [];
    mount(ADMIN, {
      socialAccountRequests: async () => ok(REQUESTS),
      socialCreateAccount: async (id) => {
        created.push(id);
        return ok(null);
      },
    });
    fireEvent.click(tab("requests"));
    expect(await screen.findAllByText("NotchFan")).not.toHaveLength(0);
    fireEvent.click(screen.getAllByText("Kefir")[0]!);
    expect(screen.getByText(i18n.t("social.admin.requests.noNote"))).toBeTruthy();
    fireEvent.click(screen.getAllByText("NotchFan")[0]!);

    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.admin.requests.create") }),
    );
    await waitFor(() => expect(created).toEqual([REQUESTS[0]!.id]));
    expect(
      useToastStore.getState().toasts.map((x) => x.messageKey),
    ).toContain("social.admin.requests.createdToast");
    // The handled request leaves the list (the query may lag behind).
    await waitFor(() =>
      expect(screen.queryAllByText("NotchFan")).toHaveLength(0),
    );
  });

  it("drops a rejected request without creating an account", async () => {
    const rejected: string[] = [];
    const created: string[] = [];
    mount(ADMIN, {
      socialAccountRequests: async () => ok(REQUESTS),
      socialRejectAccount: async (id) => {
        rejected.push(id);
        return ok(null);
      },
      socialCreateAccount: async (id) => {
        created.push(id);
        return ok(null);
      },
    });
    fireEvent.click(tab("requests"));
    fireEvent.click((await screen.findAllByText("NotchFan"))[0]!);
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.admin.requests.reject") }),
    );
    await waitFor(() => expect(rejected).toEqual([REQUESTS[0]!.id]));
    expect(created).toEqual([]);
    await waitFor(() =>
      expect(screen.queryAllByText("NotchFan")).toHaveLength(0),
    );
    expect(
      useToastStore.getState().toasts.map((x) => x.messageKey),
    ).toContain("social.admin.requests.rejectedToast");
  });

  it("shows a taken login next to the request", async () => {
    mount(ADMIN, {
      socialAccountRequests: async () => ok(REQUESTS),
      socialCreateAccount: async () => ({
        status: "error" as const,
        error: {
          code: "conflict" as const,
          message: "username is taken",
          details: { reason: "usernameTaken" },
        },
      }),
    });
    fireEvent.click(tab("requests"));
    fireEvent.click(
      await screen.findByRole("button", { name: i18n.t("social.admin.requests.create") }),
    );
    const message = i18n.t("errors.reason.usernameTaken");
    const alerts = await screen.findAllByText(message);
    expect(alerts.length).toBeGreaterThanOrEqual(2);
  });

  it("shows an empty state without requests", async () => {
    mount(ADMIN, { socialAccountRequests: async () => ok([]) });
    fireEvent.click(tab("requests"));
    expect(
      await screen.findByText(i18n.t("social.admin.requests.empty")),
    ).toBeTruthy();
    expect(
      screen.getByText(i18n.t("social.admin.requests.select")),
    ).toBeTruthy();
  });
});

