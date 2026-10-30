// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useSocialStore } from "../../stores/socialStore";
import { SOCIAL_PEER } from "../../mocks/social";
import { ChatScreen } from "./ChatScreen";
import i18n from "../../i18n";

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

beforeEach(() => useSocialStore.getState().reset());
afterEach(cleanup);
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
