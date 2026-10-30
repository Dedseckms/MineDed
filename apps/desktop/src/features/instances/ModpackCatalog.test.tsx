// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { useModpackStore } from "../../stores/modpackStore";
import { useNavStore } from "../../stores/navStore";

afterEach(cleanup);

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

async function openCatalog() {
  await act(async () => {
    useModpackStore.getState().reset();
    useNavStore.getState().go("instances");
    useNavStore.getState().setLibraryTab("mine");
  });
  renderApp();
  fireEvent.click(await screen.findByRole("tab", { name: "Modpack catalog" }));
}

describe("modpack catalog (mocks)", () => {
  it("keeps import from file and installs a pack, then opens the new instance", async () => {
    await openCatalog();
    expect(screen.getByRole("button", { name: "Import from file" })).toBeTruthy();
    const list = await screen.findByRole("list", { name: "Modpack catalog" });
    const title = await within(list).findByRole("button", { name: "Fabulously Optimized" });
    fireEvent.click(title);
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findAllByRole("button", { name: /^Install version/ })).not.toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Install latest" }));
    expect(await screen.findByRole("button", { name: /All instances/ }, { timeout: 8000 })).toBeTruthy();
    expect(useNavStore.getState().openInstance).toMatch(/^inst-fabulously-optimized/);
  }, 15000);

  it("switches provider, previews versions and installs the actual CurseForge pack", async () => {
    await openCatalog();
    await screen.findByRole("list", { name: "Modpack catalog" });
    fireEvent.change(screen.getByRole("combobox", { name: "Provider" }), { target: { value: "curseForge" } });
    const title = await screen.findByRole("button", { name: "Better MC (CurseForge)" });
    expect(useModpackStore.getState().source).toBe("curseForge");
    fireEvent.click(title);
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findAllByRole("button", { name: /^Install version/ });
    fireEvent.click(within(dialog).getByRole("button", { name: "Install latest" }));
    await waitFor(() => expect(useNavStore.getState().openInstance).toMatch(/^inst-better-mc-curseforge/), { timeout: 8000 });
  }, 15000);
});
