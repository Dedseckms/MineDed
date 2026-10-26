// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import "../i18n";
import { App } from "../App";
import { api, unwrap } from "../api/client";
import { setMockFilesPicker } from "../api/dialog";
import { handleDeepLink } from "../hooks/useDeepLinks";
import i18n from "../i18n";
import { BACKGROUND_IDS, type BackgroundId } from "../lib/backdrop";
import { useNavStore } from "../stores/navStore";
import { AppBackground, FADE_MS, nextLayers } from "./appearance/backgrounds";
import { BACKGROUND_SAVE_MS, BackgroundPicker } from "./settings/SettingsScreen";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setMockFilesPicker(null);
});
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
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

const ids = (n: number): BackgroundId[] => Array.from({ length: n }, (_, i) => BACKGROUND_IDS[i % BACKGROUND_IDS.length] as BackgroundId);

describe("background switching", () => {
  it("nextLayers keeps only the outgoing and the new scene", () => {
    expect(nextLayers("plains", "forest", true)).toEqual({ prev: "plains", cur: "forest" });
    expect(nextLayers("plains", "forest", false)).toEqual({ prev: null, cur: "forest" });
    expect(nextLayers("forest", "forest", true)).toEqual({ prev: null, cur: "forest" });
  });

  it("30 quick switches never mount more than two scenes", async () => {
    const { container, rerender } = render(<AppBackground id="plains" animate />);
    let max = 0;
    for (const id of ids(30)) {
      rerender(<AppBackground id={id} animate />);
      max = Math.max(max, container.querySelectorAll("[data-background]").length);
    }
    expect(max).toBeLessThanOrEqual(2);
    await waitFor(() => expect(container.querySelectorAll("[data-background]").length).toBe(1), { timeout: FADE_MS + 500 });
    const last = ids(30).at(-1);
    expect(container.querySelector(`[data-background="${last}"]`)).toBeTruthy();
  });

  it("reduced motion swaps instantly (one scene)", () => {
    const { container, rerender } = render(<AppBackground id="plains" animate={false} />);
    rerender(<AppBackground id="nether" animate={false} />);
    expect(container.querySelectorAll("[data-background]").length).toBe(1);
  });

  it("30 quick clicks in the picker save once, with the last choice, and stay fast", () => {
    vi.useFakeTimers();
    const onChange = vi.fn();
    render(<BackgroundPicker value={null} onChange={onChange} />);
    const radios = BACKGROUND_IDS.map((id) => screen.getByRole("radio", { name: i18n.t(`settings.backgrounds.${id}`) }));
    const t0 = performance.now();
    for (let i = 0; i < 30; i++) fireEvent.click(radios[i % radios.length] as HTMLElement);
    const elapsed = performance.now() - t0;
    const last = BACKGROUND_IDS[29 % BACKGROUND_IDS.length];
    expect(screen.getByRole("radio", { name: i18n.t(`settings.backgrounds.${last}`) }).getAttribute("aria-checked")).toBe("true");
    expect(onChange).not.toHaveBeenCalled();
    act(() => void vi.advanceTimersByTime(BACKGROUND_SAVE_MS));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(last);
    // Generous bound for slow CI; the old picker re-rendered 8 SVG scenes per click.
    expect(elapsed).toBeLessThan(3000);
  });
});

describe("local mod dependencies", () => {
  it("adding a jar checks deps; install goes through the plan dialog; manual check works", async () => {
    setMockFilesPicker(async () => ["C:\\Users\\me\\Downloads\\old-addon-1.0.jar"]);
    await act(async () => useNavStore.getState().showInstance("inst-fabric", "mods"));
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("content.addLocal") }));
    let dialog = await screen.findByRole("dialog", { name: i18n.t("content.localDeps.title") });
    expect(within(dialog).getByText("old-addon-1.0.jar")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Install Sodium" }));
    expect(await screen.findByRole("dialog", { name: i18n.t("content.plan.title") })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.cancel") }));

    fireEvent.click(await screen.findByRole("button", { name: i18n.t("content.localDeps.check") }));
    dialog = await screen.findByRole("dialog", { name: i18n.t("content.localDeps.title") });
    expect(within(dialog).getAllByTestId("local-deps-file").length).toBeGreaterThan(0);
    fireEvent.click(within(dialog).getByRole("button", { name: /^Install everything needed/ }));
    expect(await screen.findByRole("dialog", { name: i18n.t("content.plan.title") })).toBeTruthy();
  });
});

describe("deep links and invites", () => {
  it("join/<code> opens Multiplayer with the code; invite/<code> opens registration", async () => {
    await act(async () => useNavStore.getState().go("play"));
    renderApp();
    await act(async () => void handleDeepLink("mineup://join/ABCD-1234"));
    const code = (await screen.findByTestId("join-code")) as HTMLInputElement;
    expect(code.value).toBe("ABCD-1234");
    expect(useNavStore.getState().pendingJoin).toBeNull();

    await act(async () => void handleDeepLink("mineded://invite/inv-5678"));
    const invite = (await screen.findByTestId("invite-input")) as HTMLInputElement;
    await waitFor(() => expect(invite.value).toBe("inv-5678"));
    expect(useNavStore.getState().section).toBe("accounts");
    expect(handleDeepLink("mineded://unknown/x")).toBe(false);
  });

  it("Paste reads an invite URL from the clipboard", async () => {
    const readText = vi.fn(async () => "https://skin.example/web/registration?invite=Clip_1234");
    Object.defineProperty(navigator, "clipboard", { value: { readText }, configurable: true });
    await act(async () => useNavStore.getState().go("accounts"));
    renderApp();
    fireEvent.click(await screen.findByRole("radio", { name: i18n.t("accounts.yggdrasil.register") }));
    fireEvent.click(await screen.findByRole("button", { name: i18n.t("accounts.yggdrasil.paste") }));
    const invite = (await screen.findByTestId("invite-input")) as HTMLInputElement;
    await waitFor(() => expect(invite.value).toBe("Clip_1234"));

    readText.mockRejectedValueOnce(new Error("denied"));
    fireEvent.click(screen.getByRole("button", { name: i18n.t("accounts.yggdrasil.paste") }));
    expect(await screen.findByText(i18n.t("accounts.yggdrasil.pasteFailed"))).toBeTruthy();
  });
});
