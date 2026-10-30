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
import type { SocialUser } from "../../bindings";

const h = vi.hoisted(() => ({ api: null as unknown as Commands }));
// happy-dom's WAAPI rejects `finished` on cancel; force Motion's JS path.
for (const proto of [Element.prototype, HTMLElement.prototype])
  Reflect.deleteProperty(proto, "animate");
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
const { useSocialStore } = await import("../../stores/socialStore");
const { useTaskStore } = await import("../../stores/taskStore");
const { ProfileEditor } = await import("./ProfileEditor");
const i18n = (await import("../../i18n")).default;

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const SELF: SocialUser = {
  uuid: "1".repeat(32),
  name: "Steve",
  skinUrl: null,
  status: "online",
  role: "user",
  isFriend: false,
  canMessage: true,
  canInvite: true,
  displayName: "Steve the Miner",
  bio: "hi",
  accent: "violet",
};

function mount(
  user: SocialUser = SELF,
  extra: Partial<Commands> = {},
  onClose = () => {},
) {
  h.api = createMockCommands(extra, { bus: createMockBus(), tickMs: 5 });
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ProfileEditor self={user} onClose={onClose} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  useSocialStore.getState().reset();
  useTaskStore.getState().reset();
});
afterEach(() => {
  cleanup();
});

describe("profile editor", () => {
  it("starts from the current profile", () => {
    mount();
    expect(
      (
        screen.getByLabelText(
          i18n.t("social.profile.displayName"),
        ) as HTMLInputElement
      ).value,
    ).toBe("Steve the Miner");
    expect(
      (
        screen.getByLabelText(i18n.t("social.profile.bio")) as HTMLTextAreaElement
      ).value,
    ).toBe("hi");
    expect(
      screen
        .getByRole("radio", { name: i18n.t("social.accent.violet") })
        .getAttribute("aria-checked"),
    ).toBe("true");
    expect(screen.getByText("2 / 120")).toBeTruthy();
  });

  it("blocks saving when the display name or bio is too long", () => {
    mount();
    const name = screen.getByLabelText(i18n.t("social.profile.displayName"));
    fireEvent.change(name, { target: { value: "a".repeat(25) } });
    expect(screen.getByText(i18n.t("social.profile.nameTooLong"))).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: i18n.t("common.save") }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    fireEvent.change(name, { target: { value: "Alex" } });
    fireEvent.change(screen.getByLabelText(i18n.t("social.profile.bio")), {
      target: { value: "a".repeat(121) },
    });
    expect(screen.getByText(i18n.t("social.profile.bioTooLong"))).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: i18n.t("common.save") }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("saves name, bio and accent in one action", async () => {
    const update = vi.fn(async (patch: Parameters<Commands["socialProfileUpdate"]>[0]) =>
      ok({ ...SELF, displayName: patch.displayName ?? null, bio: patch.bio ?? null, accent: patch.accent ?? null }),
    );
    const onClose = vi.fn();
    mount(SELF, { socialProfileUpdate: update }, onClose);
    fireEvent.change(screen.getByLabelText(i18n.t("social.profile.displayName")), {
      target: { value: "Alex" },
    });
    fireEvent.change(screen.getByLabelText(i18n.t("social.profile.bio")), {
      target: { value: "" },
    });
    fireEvent.click(
      screen.getByRole("radio", { name: i18n.t("social.accent.amber") }),
    );
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.save") }));
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        displayName: "Alex",
        bio: "",
        accent: "amber",
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(useSocialStore.getState().self?.displayName).toBe("Alex");
    expect(useSocialStore.getState().self?.accent).toBe("amber");
  });

  it("removes the avatar", async () => {
    const clear = vi.fn(async () => ok({ ...SELF, displayName: null }));
    mount(SELF, { socialAvatarClear: clear });
    fireEvent.click(
      screen.getByRole("button", {
        name: i18n.t("social.profile.avatarRemove"),
      }),
    );
    await waitFor(() => expect(clear).toHaveBeenCalled());
    await waitFor(() =>
      expect(useSocialStore.getState().self?.displayName).toBeNull(),
    );
  });

  it("uploads a picked image through the task channel", async () => {
    const set = vi.fn(
      async (
        _path: string,
        _channel: Parameters<Commands["socialAvatarSet"]>[1],
      ) => ok("task-1"),
    );
    mount(SELF, { socialAvatarSet: set });
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("social.profile.avatarPick") }),
    );
    await waitFor(() => expect(set).toHaveBeenCalled());
    expect(set.mock.calls[0]?.[0]).toMatch(/skin\.png$/);
  });
});
