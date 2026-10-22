// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { mockBus } from "../../mocks";
import { useNavStore } from "../../stores/navStore";

afterEach(cleanup);

// See App.test.tsx: force Motion's JS animation path under happy-dom.
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
  // The mock settings say "ru"; the shell applies them, so switch them to English.
  const settings = unwrap(await api.settingsGet());
  unwrap(await api.settingsSet({ ...settings, language: "en" }));
  await i18n.changeLanguage("en");
});

function renderApp() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <App />
    </QueryClientProvider>,
  );
}

describe("VPN screen (mocks)", () => {
  it("subscription by link → nodes → mode → ping → select", async () => {
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    expect(await screen.findByText("No nodes yet")).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Launcher + game" }).hasAttribute("disabled")).toBe(true);

    const urls = screen.getByLabelText("Subscription links");
    fireEvent.change(urls, { target: { value: "ftp://bad" } });
    fireEvent.click(screen.getByRole("button", { name: "Save and refresh" }));
    expect(await screen.findByText(i18n.t("validation.url.scheme"))).toBeTruthy();

    fireEvent.change(urls, { target: { value: "https://sub.example/abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Save and refresh" }));
    expect(await screen.findByText(/Amsterdam/, {}, { timeout: 4000 })).toBeTruthy();
    expect(screen.getByRole("heading", { name: /Nodes \(9\)/ })).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: "Launcher + game" }));
    expect(await screen.findByText("On")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Ping all" }));
    expect((await screen.findAllByText(/^\d+ ms$/)).length).toBeGreaterThan(0);

    const group = screen.getByRole("radiogroup", { name: "VPN nodes" });
    const helsinki = within(group).getByRole("radio", { name: /Helsinki/ });
    fireEvent.click(helsinki);
    await screen.findByText((_, el) => el?.tagName === "DD" && /Helsinki/.test(el.textContent ?? ""));
    expect(within(group).getByRole("radio", { name: /Helsinki/ }).getAttribute("aria-checked")).toBe("true");
  }, 10_000);

  it("validates the subscription file path", async () => {
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    fireEvent.click(await screen.findByRole("radio", { name: "From file" }));
    const path = screen.getByLabelText("Subscription file path");
    fireEvent.change(path, { target: { value: "sub.yaml" } });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText(i18n.t("validation.path.absolute"))).toBeTruthy();
    fireEvent.change(path, { target: { value: "C:\\missing.yaml" } });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText(i18n.t("errors.notFound"))).toBeTruthy();
  });

  it("net://blocked shows a toast that opens the VPN screen", async () => {
    await act(async () => useNavStore.getState().go("play"));
    renderApp();
    await screen.findByRole("heading", { level: 1, name: "Play" });
    await act(async () => {
      // The app subscribes lazily; give the mock bus subscription a tick.
      await new Promise((r) => setTimeout(r, 0));
      mockBus.emit("netBlocked", { host: "cdn.example", suggestionKey: "net.blockedHint" });
    });
    expect(await screen.findByText(i18n.t("net.blocked", { host: "cdn.example" }))).toBeTruthy();
    expect(screen.getByText(i18n.t("net.blockedHint"))).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open VPN" }));
    expect((await screen.findByRole("heading", { level: 1 })).textContent).toBe("VPN");
  });
});
