// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { MOCK_UPDATE, mockBus, mockUpdateControl } from "../../mocks";
import { useNavStore } from "../../stores/navStore";
import { useTaskStore } from "../../stores/taskStore";
import { useToastStore } from "../../stores/toastStore";
import { useUpdateStore } from "../../stores/updateStore";

afterEach(cleanup);

// See App.test.tsx: force Motion's JS animation path under happy-dom.
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
  const settings = unwrap(await api.settingsGet());
  unwrap(await api.settingsSet({ ...settings, language: "en", autoUpdate: true }));
  await i18n.changeLanguage("en");
});

beforeEach(() => {
  useUpdateStore.getState().reset();
  useTaskStore.getState().reset();
  useToastStore.getState().reset();
  mockUpdateControl.next = null;
  mockUpdateControl.installError = null;
});

async function setAutoUpdate(autoUpdate: boolean) {
  const settings = unwrap(await api.settingsGet());
  unwrap(await api.settingsSet({ ...settings, autoUpdate }));
}

function renderApp() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <App />
    </QueryClientProvider>,
  );
}

const availableText = () => i18n.t("updates.available", { version: MOCK_UPDATE.version });

describe("launcher self-update (mocks)", () => {
  it("startup check shows a banner with notes; Later hides it", async () => {
    mockUpdateControl.next = { ...MOCK_UPDATE };
    await act(async () => useNavStore.getState().go("play"));
    renderApp();
    const banner = await screen.findByTestId("update-banner");
    expect(within(banner).getByText(availableText())).toBeTruthy();
    expect(within(banner).getByText(/Faster instance launch/)).toBeTruthy();
    expect(within(banner).getByText(i18n.t("updates.current", { version: MOCK_UPDATE.currentVersion }))).toBeTruthy();
    fireEvent.click(within(banner).getByRole("button", { name: "Later" }));
    await waitFor(() => expect(screen.queryByTestId("update-banner")).toBeNull());
    expect(useUpdateStore.getState().dismissed).toBe(MOCK_UPDATE.version);
  });

  it("no startup check when autoUpdate is off, but update://available still shows the banner", async () => {
    await setAutoUpdate(false);
    mockUpdateControl.next = { ...MOCK_UPDATE };
    try {
      await act(async () => useNavStore.getState().go("play"));
      renderApp();
      await screen.findByRole("heading", { level: 1, name: "Play" });
      await act(async () => new Promise((r) => setTimeout(r, 20)));
      expect(useUpdateStore.getState().autoChecked).toBe(false);
      expect(screen.queryByTestId("update-banner")).toBeNull();
      await act(async () => mockBus.emit("updateAvailable", { ...MOCK_UPDATE }));
      expect(await screen.findByTestId("update-banner")).toBeTruthy();
    } finally {
      await setAutoUpdate(true);
    }
  });

  it("Update runs the appUpdate task through the task center", async () => {
    mockUpdateControl.next = { ...MOCK_UPDATE };
    await act(async () => useNavStore.getState().go("play"));
    renderApp();
    const banner = await screen.findByTestId("update-banner");
    fireEvent.click(within(banner).getByRole("button", { name: "Update" }));
    await waitFor(() => expect(useUpdateStore.getState().installTask).not.toBeNull());
    const id = useUpdateStore.getState().installTask ?? "";
    expect(useTaskStore.getState().tasks[id]?.kind).toBe("appUpdate");
    await waitFor(() => expect(useTaskStore.getState().tasks[id]?.status).toBe("succeeded"), { timeout: 5000 });
  }, 10_000);

  it("a failed install raises a toast and offers a retry", async () => {
    mockUpdateControl.next = { ...MOCK_UPDATE };
    mockUpdateControl.installError = { code: "hashMismatch", message: "mock: bad signature", details: null };
    await act(async () => useNavStore.getState().go("play"));
    renderApp();
    const banner = await screen.findByTestId("update-banner");
    fireEvent.click(within(banner).getByRole("button", { name: "Update" }));
    expect(await screen.findByText(i18n.t("updates.installFailed"), {}, { timeout: 5000 })).toBeTruthy();
    expect(within(banner).getByRole("button", { name: "Retry" })).toBeTruthy();
  }, 10_000);

  it("settings: version, up to date, available, network error", async () => {
    await act(async () => useNavStore.getState().go("settings"));
    renderApp();
    expect((await screen.findByTestId("app-version")).textContent).toContain("0.0.0-mock");
    const check = screen.getByRole("button", { name: "Check for updates" });

    fireEvent.click(check);
    expect(await screen.findByText(i18n.t("updates.upToDate"))).toBeTruthy();

    mockUpdateControl.next = { code: "network", message: "mock: offline", details: null };
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(i18n.t("updates.checkFailed"))).toBeTruthy();
    expect(screen.getByText(i18n.t("errors.network"))).toBeTruthy();

    mockUpdateControl.next = { ...MOCK_UPDATE };
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByTestId("update-banner")).toBeTruthy();
    expect(screen.getAllByText(availableText()).length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText(i18n.t("updates.checkFailed"))).toBeNull();
  });
});
