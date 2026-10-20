// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import "./i18n";
import { App } from "./App";
import i18n from "./i18n";
import { useNavStore } from "./stores/navStore";

afterEach(cleanup);

// happy-dom's WAAPI rejects `finished` on cancel, which Motion leaves unhandled.
// Force Motion's JS animation path in this DOM emulation.
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

function renderApp() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <App />
    </QueryClientProvider>,
  );
}

describe("App shell (mocks)", () => {
  it("renders every section without crashing", async () => {
    await i18n.changeLanguage("en");
    renderApp();
    expect(await screen.findByRole("option", { name: /1\.21\.10/ })).toBeTruthy();
    expect(await screen.findAllByText("Steve_42")).not.toHaveLength(0);

    for (const section of ["instances", "accounts", "settings", "play"] as const) {
      await act(async () => useNavStore.getState().go(section));
      const heading = await screen.findByRole("heading", { level: 1 });
      expect(heading.textContent).toBe(i18n.t(`nav.${section}`));
    }
  });

  it("validates the offline nickname inline", async () => {
    await i18n.changeLanguage("en");
    await act(async () => useNavStore.getState().go("accounts"));
    renderApp();
    const input = await screen.findByLabelText("Nickname");
    fireEvent.change(input, { target: { value: "bad name" } });
    expect(await screen.findByText(i18n.t("validation.nick.chars"))).toBeTruthy();
  });
});
