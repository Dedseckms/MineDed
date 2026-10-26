// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { useNavStore } from "../../stores/navStore";
import { useRoomStore } from "../../stores/roomStore";

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

async function confirmIn(dialogTitle: string, button: string) {
  const dialog = await screen.findByRole("dialog", { name: dialogTitle });
  fireEvent.click(within(dialog).getByRole("button", { name: button }));
}

describe("Multiplayer screen (mocks)", () => {
  it("host: open world → code/link → friends with path badges → chat → kick → close", async () => {
    await act(async () => useNavStore.getState().go("multiplayer"));
    renderApp();
    fireEvent.click(await screen.findByRole("radio", { name: "Open" }));
    const pw = screen.getAllByLabelText(/Room password/)[0] as HTMLElement;
    fireEvent.change(pw, { target: { value: "abc" } });
    expect(await screen.findByText(i18n.t("validation.room.password"))).toBeTruthy();
    fireEvent.change(pw, { target: { value: "" } });
    await screen.findAllByRole("option", { name: /1\.21/ });
    fireEvent.click(screen.getByRole("button", { name: "Create room" }));

    const code = await screen.findByTestId("room-code");
    expect(code.textContent).toMatch(/^[A-Z0-9]{4}-[A-Z0-9]{4}$/);
    expect(screen.getByRole("button", { name: "Copy link" })).toBeTruthy();
    expect(screen.getByText("You are the host")).toBeTruthy();
    expect(screen.queryByTestId("room-local-addr")).toBeNull();
    // Code is shown at once; the world-open banner flips when the LAN port is found.
    expect(screen.getByTestId("world-closed").textContent).toContain("Open to LAN");
    expect(await screen.findByTestId("world-open", {}, { timeout: 4000 })).toBeTruthy();
    expect(screen.queryByTestId("world-closed")).toBeNull();

    expect(await screen.findByText("AlexBuilder", { selector: "p" }, { timeout: 4000 })).toBeTruthy();
    expect(await screen.findByText("Direct", {}, { timeout: 4000 })).toBeTruthy();
    expect(await screen.findByText("Relay", {}, { timeout: 6000 })).toBeTruthy();
    expect(screen.getAllByText(/^Ping \d+ ms$/).length).toBe(2);

    const input = screen.getByLabelText("Message");
    fireEvent.change(input, { target: { value: "hello all" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByText("hello all")).toBeTruthy();
    expect(screen.getByText("You")).toBeTruthy();
    expect(useRoomStore.getState().unread).toBe(0);

    fireEvent.click(screen.getByRole("button", { name: "Kick AlexBuilder" }));
    await confirmIn("Kick the player?", "Kick");
    await screen.findByRole("heading", { name: "Players (1)" });

    fireEvent.click(screen.getByRole("button", { name: "Close room" }));
    await confirmIn("Close the room?", "Close room");
    expect(await screen.findByRole("button", { name: "Create room" })).toBeTruthy();
  }, 15_000);

  it("guest: join by link with a password → local address", async () => {
    await act(async () => useNavStore.getState().go("multiplayer"));
    renderApp();
    const codeInput = await screen.findByLabelText("Code or link");
    fireEvent.change(codeInput, { target: { value: "https://evil.example" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    expect(await screen.findByText(i18n.t("validation.room.link"))).toBeTruthy();

    fireEvent.change(codeInput, { target: { value: "mineup://join/lock-1234" } });
    await screen.findAllByRole("option", { name: /1\.21/ });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    expect(await screen.findByText("Wrong room password.")).toBeTruthy();

    const pw = screen.getAllByLabelText(/Room password/)[1] as HTMLElement;
    fireEvent.change(pw, { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    const addr = await screen.findByTestId("room-local-addr");
    expect(addr.textContent).toMatch(/^127\.0\.0\.1:\d+$/);
    expect(screen.getByText("You are a guest")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Kick/ })).toBeNull();
    expect(screen.getByText("Steve_Host")).toBeTruthy();
    expect(screen.getByTestId("server-added").textContent).toContain("MineDed — Steve_Host");
    expect(screen.queryByTestId("world-closed")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Leave" }));
    await confirmIn("Leave the room?", "Leave");
    expect(await screen.findByLabelText("Code or link")).toBeTruthy();
  });
});
