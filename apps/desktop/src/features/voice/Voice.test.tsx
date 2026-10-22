// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { useNavStore } from "../../stores/navStore";
import { useVoiceStore } from "../../stores/voiceStore";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

// See App.test.tsx: force Motion's JS animation path under happy-dom.
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

describe("voice room (mocks)", () => {
  it("create voice room → join voice → speakers, per-peer mute/volume, self mute → leave", async () => {
    await act(async () => useNavStore.getState().go("multiplayer"));
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "Create voice room" }));

    expect(await screen.findByRole("heading", { name: "Your voice room" })).toBeTruthy();
    expect(screen.getByText("Voice only")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Go to Play" })).toBeNull();
    expect(screen.getByTestId("voice-status").textContent).toBe("Voice is off");

    expect(await screen.findByText("AlexBuilder", {}, { timeout: 4000 })).toBeTruthy();
    await screen.findByText("Direct", {}, { timeout: 4000 });

    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await waitFor(() => expect(screen.getByTestId("voice-status").textContent).toBe("Push to talk: V"));
    expect(screen.getByRole("meter", { name: "Your microphone level" })).toBeTruthy();

    const volume = await screen.findByRole("slider", { name: "Volume of AlexBuilder" });
    fireEvent.change(volume, { target: { value: "150" } });
    const alexId = unwrap(await api.roomState())?.peers.find((p) => p.name === "AlexBuilder")?.peerId ?? "";
    await waitFor(async () => expect(unwrap(await api.voiceSettings()).peerVolumes).toEqual({ [alexId]: 1.5 }), { timeout: 2000 });

    fireEvent.click(screen.getByRole("button", { name: "Mute AlexBuilder" }));
    expect(await screen.findByRole("button", { name: "Unmute AlexBuilder" })).toBeTruthy();

    await waitFor(() => expect(document.querySelector("[data-speaking]")).not.toBeNull(), { timeout: 4000 });

    const muteButtons = screen.getAllByRole("button", { name: "Mute microphone" });
    expect(muteButtons.length).toBe(2); // panel + header shortcut
    fireEvent.click(muteButtons[0] as HTMLElement);
    await waitFor(() => expect(screen.getByTestId("voice-status").textContent).toBe("Microphone muted"));

    fireEvent.click(screen.getAllByRole("button", { name: "Unmute microphone", pressed: true })[1] as HTMLElement); // header
    await waitFor(() => expect(screen.getByTestId("voice-status").textContent).toBe("Push to talk: V"));

    fireEvent.click(screen.getByRole("button", { name: "Leave voice" }));
    await waitFor(() => expect(screen.getByTestId("voice-status").textContent).toBe("Voice is off"));
    expect(unwrap(await api.voiceStatus()).active).toBe(false);
    expect(useVoiceStore.getState().speaking).toEqual({});

    fireEvent.click(screen.getByRole("button", { name: "Close room" }));
    const dialog = await screen.findByRole("dialog", { name: "Close the room?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Close room" }));
    expect(await screen.findByRole("button", { name: "Create voice room" })).toBeTruthy();
  }, 20_000);

  it("join form: voice-only hides the instance and joins without one", async () => {
    await act(async () => useNavStore.getState().go("multiplayer"));
    renderApp();
    fireEvent.change(await screen.findByLabelText("Code or link"), { target: { value: "voxa-7777" } });
    fireEvent.click(screen.getByRole("radio", { name: "Voice only" }));
    expect(screen.getAllByLabelText("Instance")).toHaveLength(1); // only the "create" form
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    expect(await screen.findByRole("heading", { name: "Voice room" })).toBeTruthy();
    expect(screen.queryByTestId("room-local-addr")).toBeNull();
    unwrap(await api.roomLeave());
  });
});

describe("voice settings (mocks)", () => {
  it("mode, PTT key capture, device, mic test meter", async () => {
    await act(async () => useNavStore.getState().go("settings"));
    renderApp();
    const capture = await screen.findByTestId("ptt-capture");
    expect(capture.textContent).toBe("V");
    fireEvent.click(capture);
    expect(capture.textContent).toBe("Press a key…");
    fireEvent.keyDown(window, { code: "Escape", key: "Escape" });
    await waitFor(() => expect(screen.getByTestId("ptt-capture").textContent).toBe("V"));
    fireEvent.click(screen.getByTestId("ptt-capture"));
    fireEvent.keyDown(window, { code: "KeyB", key: "b" });
    await waitFor(() => expect(screen.getByTestId("ptt-capture").textContent).toBe("B"));
    await waitFor(async () => expect(unwrap(await api.voiceSettings()).pttKey).toBe("KeyB"));

    fireEvent.change(screen.getByLabelText("Microphone"), { target: { value: "in-usb" } });
    await waitFor(async () => expect(unwrap(await api.voiceSettings()).inputDevice).toBe("in-usb"));

    fireEvent.click(screen.getByRole("radio", { name: "Voice activation" }));
    await waitFor(() => expect(screen.queryByTestId("ptt-capture")).toBeNull());

    fireEvent.click(screen.getByRole("button", { name: "Check microphone" }));
    const meter = screen.getByRole("meter", { name: "Microphone level" });
    await waitFor(() => expect(Number(meter.getAttribute("aria-valuenow"))).toBeGreaterThan(0), { timeout: 3000 });
    fireEvent.click(screen.getByRole("button", { name: "Stop check" }));
    await waitFor(async () => expect(unwrap(await api.voiceStatus()).micTest).toBe(false));
    expect(await screen.findByRole("button", { name: "Check microphone" })).toBeTruthy();
  }, 10_000);
});

describe("overlay route", () => {
  it("renders the compact overlay instead of the shell", async () => {
    window.location.hash = "#/overlay";
    renderApp();
    expect(await screen.findByText("Not in a room")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close overlay" })).toBeTruthy();
    expect(screen.getByRole("list", { name: "Participants" })).toBeTruthy();
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(document.documentElement.dataset.overlay).toBe("true");
  });
});
