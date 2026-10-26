// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import { setMockFilePicker } from "../../api/dialog";
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

/** The `<section>` of a screen `Section` by its heading. */
function section(title: string): HTMLElement {
  const el = screen.getByRole("heading", { level: 2, name: title }).closest("section");
  if (!el) throw new Error(`no section ${title}`);
  return el;
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

  it("imports a subscription file through the file dialog", async () => {
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    fireEvent.click(await screen.findByRole("radio", { name: "From file" }));
    const sub = section("Subscription");
    setMockFilePicker(async () => null);
    fireEvent.click(within(sub).getByRole("button", { name: "Choose file…" }));
    setMockFilePicker(async () => "C:\\missing.yaml");
    fireEvent.click(within(sub).getByRole("button", { name: "Choose file…" }));
    expect(await screen.findByText(i18n.t("errors.notFound"))).toBeTruthy();
    setMockFilePicker(null);
  });

  it("MineDed VPN: connect with an own-server account, usage, turn off", async () => {
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    expect(await screen.findByText("MineDedPlayer")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Connect to MineDed VPN" }));
    expect(await screen.findByText("Connected")).toBeTruthy();
    expect(screen.getByText(/^256 GB left until /)).toBeTruthy();
    expect(screen.getByRole("progressbar", { name: "Traffic used" })).toBeTruthy();
    expect(unwrap(await api.vpnState()).mode).toBe("launcherAndGame");
    expect(unwrap(await api.vpnNodes()).map((n) => n.name)).toEqual(["MineDed NL"]);
    fireEvent.click(screen.getByRole("button", { name: "Turn off" }));
    expect(await screen.findByRole("button", { name: "Connect to MineDed VPN" })).toBeTruthy();
  }, 10_000);

  it("apps editor: presets, running programs, file, remove, save", async () => {
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    expect(await screen.findByText(i18n.t("vpn.apps.empty"))).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save list" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Add Telegram" }));
    expect(screen.getByRole("button", { name: "Add Telegram" }).hasAttribute("disabled")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "From running" }));
    const dialog = await screen.findByRole("dialog", { name: "Running programs" });
    fireEvent.change(within(dialog).getByLabelText("Search programs"), { target: { value: "spot" } });
    fireEvent.click(await within(dialog).findByRole("button", { name: "Add Spotify" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));

    setMockFilePicker(async () => "C:\\Games\\Tools\\OBS.exe");
    fireEvent.click(within(section("Programs through VPN")).getByRole("button", { name: "Choose file…" }));
    expect(await screen.findByText("OBS.exe")).toBeTruthy();
    setMockFilePicker(null);

    fireEvent.click(screen.getByRole("button", { name: "Remove Spotify" }));
    fireEvent.click(save);
    await waitFor(async () =>
      expect(unwrap(await api.vpnApps()).map((a) => a.process)).toEqual(["Telegram.exe", "OBS.exe"]),
    );
    await waitFor(() => expect(screen.getByRole("button", { name: "Save list" }).hasAttribute("disabled")).toBe(true));
  }, 10_000);

  it("apps mode shows the admin-rights note and a friendly UAC error", async () => {
    unwrap(await api.vpnSetApps([{ name: "Declined", process: "decline.exe", path: null }]));
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    fireEvent.click(await screen.findByRole("radio", { name: "Selected programs" }));
    expect(await screen.findByText(i18n.t("vpn.mode.adminNote"))).toBeTruthy();
    expect(await screen.findByText(i18n.t("errors.reason.uacDeclined"))).toBeTruthy();
    unwrap(await api.vpnSetApps([]));
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
