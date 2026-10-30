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
import type { SocialReport, SocialUser } from "../../bindings";

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

  it("shows the full section to admins with four tabs", () => {
    mount();
    expect(screen.getByText(i18n.t("social.admin.title"))).toBeTruthy();
    for (const value of ["moderation", "users", "server", "journal"])
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
