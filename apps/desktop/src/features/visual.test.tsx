// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import "../i18n";
import { api, unwrap } from "../api/client";
import { qk } from "../api/queries";
import { ErrorMessage } from "../components/ui/feedback";
import { usePttSoundCue } from "../hooks/usePttSoundCue";
import i18n from "../i18n";
import { useNavStore } from "../stores/navStore";
import { usePrefsStore } from "../stores/prefsStore";
import { useVoiceStore } from "../stores/voiceStore";
import { BackgroundScene } from "./appearance/backgrounds";
import { BACKGROUND_IDS } from "../lib/backdrop";
import { BorderlessHelper } from "./voice/BorderlessHelper";
import { CreateRoomForm } from "./rooms/RoomForms";

afterEach(cleanup);
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
  await i18n.changeLanguage("en");
});

function wrap(qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("ErrorMessage details", () => {
  it("shows the reason text, collapsible details and the proxy hint", async () => {
    render(<ErrorMessage error={{ code: "network", message: "x", details: { reason: "relayUnreachable", hint: "useProxy", status: "503" } }} />, {
      wrapper: wrap(),
    });
    expect(screen.getByText(i18n.t("errors.reason.relayUnreachable"))).toBeTruthy();
    expect(screen.getByTestId("proxy-hint")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(screen.getByText("503")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open VPN" }));
    expect(useNavStore.getState().section).toBe("vpn");
  });
});

describe("backgrounds", () => {
  it("render every scene as inline SVG", () => {
    for (const id of BACKGROUND_IDS) {
      const { container, unmount } = render(<BackgroundScene id={id} />);
      expect(container.querySelector(`[data-background="${id}"] svg`)).toBeTruthy();
      expect(container.querySelector("img")).toBeNull();
      unmount();
    }
  });
});

describe("rooms auth mode", () => {
  it("pre-selects Open for an offline default account and disables My server", async () => {
    render(<CreateRoomForm />, { wrapper: wrap() });
    const open = await screen.findByRole("radio", { name: "Open" });
    await screen.findByTestId("auth-mode-disabled");
    expect(open.getAttribute("aria-checked")).toBe("true");
    expect((screen.getByRole("radio", { name: "My server" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("borderless helper", () => {
  it("vanilla shows instructions, modded installs a mod", async () => {
    act(() => useNavStore.getState().selectPlayInstance("inst-survival"));
    const { unmount } = render(<BorderlessHelper />, { wrapper: wrap() });
    expect(await screen.findByTestId("borderless-vanilla")).toBeTruthy();
    unmount();

    act(() => useNavStore.getState().selectPlayInstance("inst-fabric"));
    render(<BorderlessHelper />, { wrapper: wrap() });
    fireEvent.click(await screen.findByRole("button", { name: "Borderless fullscreen" }));
    expect(await screen.findByRole("dialog", { name: i18n.t("content.plan.title") })).toBeTruthy();
    expect(await screen.findByText("Borderless Fullscreen")).toBeTruthy();
  });
});

describe("usePttSoundCue", () => {
  it("plays on push-to-talk edges and respects the preference", () => {
    const qc = new QueryClient();
    qc.setQueryData(qk.voiceStatus, { active: true, selfMuted: false, mode: "pushToTalk", micTest: false });
    const play = vi.fn();
    const { unmount } = renderHook(() => usePttSoundCue(play), { wrapper: wrap(qc) });
    act(() => useVoiceStore.getState().applySpeaking({ peerId: "self", speaking: true, level: 0.5 }));
    act(() => useVoiceStore.getState().applySpeaking({ peerId: "self", speaking: false, level: 0 }));
    expect(play.mock.calls.map((c) => c[0])).toEqual(["pttOn", "pttOff"]);
    act(() => usePrefsStore.getState().setPttSound(false));
    act(() => useVoiceStore.getState().applySpeaking({ peerId: "self", speaking: true, level: 0.5 }));
    expect(play).toHaveBeenCalledTimes(2);
    unmount();
    usePrefsStore.getState().setPttSound(true);
  });
});

it("mock catalog knows the borderless mods", async () => {
  const v = unwrap(await api.contentVersions("modrinth", "UIexcMP1", { mc: "1.21.1", loader: "fabric" }));
  expect(v.length).toBeGreaterThan(0);
});
