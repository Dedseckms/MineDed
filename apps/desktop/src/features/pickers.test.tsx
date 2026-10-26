// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../i18n";
import { App } from "../App";
import { api, unwrap } from "../api/client";
import { setMockFilePicker, setMockFilesPicker } from "../api/dialog";
import { contentAddLocal } from "../api/localContent";
import i18n from "../i18n";
import { errorView } from "../lib/errors";
import { useNavStore } from "../stores/navStore";
import { useToastStore } from "../stores/toastStore";

afterEach(() => {
  cleanup();
  setMockFilePicker(null);
  setMockFilesPicker(null);
});

for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
  const settings = unwrap(await api.settingsGet());
  unwrap(await api.settingsSet({ ...settings, language: "en" }));
  await i18n.changeLanguage("en");
});

function renderApp() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <App />
    </QueryClientProvider>,
  );
}

describe("VPN ping while the VPN is off", () => {
  it("maps the backend error to a friendly message", () => {
    const v = errorView({ code: "invalidInput", message: "VPN is not running", details: null });
    expect(v.messageKey).toBe("errors.reason.vpnNotRunning");
    expect(errorView({ code: "invalidInput", message: "other", details: null }).messageKey).toBe("errors.invalidInput");
  });

  it("disables the ping button with a hint until the VPN runs", async () => {
    unwrap(await api.vpnImportSubscriptionFile("C:\\Users\\me\\sub.yaml"));
    await act(async () => useNavStore.getState().go("vpn"));
    renderApp();
    const ping = await screen.findByRole("button", { name: "Ping all" });
    await screen.findByText(/Amsterdam/);
    expect(ping.hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("Turn on the VPN to check the ping")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: "Launcher + game" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Ping all" }).hasAttribute("disabled")).toBe(false));
    expect(screen.queryByText("Turn on the VPN to check the ping")).toBeNull();
    unwrap(await api.vpnSetMode("off"));
  });
});

describe("modpack import", () => {
  it("picks a file and shows only its name", async () => {
    await act(async () => useNavStore.getState().go("instances"));
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "Import from file" }));
    const dialog = await screen.findByRole("dialog");
    const submit = within(dialog).getByRole("button", { name: "Import" });
    expect(submit.hasAttribute("disabled")).toBe(true);

    const full = "C:\\Users\\me\\Very\\Long\\Folder\\Name\\Downloads\\Cool Pack.zip";
    setMockFilePicker(async (filters) => {
      expect(filters[0]?.extensions).toEqual(["mrpack", "zip"]);
      return full;
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Choose file…" }));
    const file = await within(dialog).findByText("Cool Pack.zip");
    expect(file.getAttribute("title")).toBe(full);
    expect(within(dialog).queryByText(full)).toBeNull();
    expect(submit.hasAttribute("disabled")).toBe(false);
    fireEvent.click(submit);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("content from disk", () => {
  it("mock content_add_local validates kinds and extensions", async () => {
    const bad = await contentAddLocal("inst-fabric", "mod", ["C:\\a\\pack.zip"]);
    expect(bad.status === "error" && bad.error.code).toBe("invalidInput");
    const modpack = await contentAddLocal("inst-fabric", "modpack", ["C:\\a\\p.mrpack"]);
    expect(modpack.status).toBe("error");
    const added = unwrap(await contentAddLocal("inst-fabric", "resourcePack", ["/home/u/My Pack.zip"]));
    expect(added).toMatchObject([{ fileName: "My Pack.zip", kind: "resourcePack", source: "local", enabled: true }]);
    expect(unwrap(await api.contentList("inst-fabric")).some((c) => c.fileName === "My Pack.zip")).toBe(true);
  });

  it("adds mods from the computer, refreshes the list and toasts the count", async () => {
    useToastStore.getState().reset();
    await act(async () => {
      useNavStore.getState().showInstance("inst-fabric");
      useNavStore.getState().setInstanceTab("mods");
    });
    renderApp();
    setMockFilesPicker(async (filters) => {
      expect(filters[0]?.extensions).toEqual(["jar"]);
      return ["C:\\Users\\me\\Downloads\\local-one.jar", "C:\\Users\\me\\Downloads\\local-two.jar"];
    });
    fireEvent.click(await screen.findByRole("button", { name: "Add from computer" }));
    expect((await screen.findAllByText(/local-two\.jar/)).length).toBeGreaterThan(0);
    expect(useToastStore.getState().toasts.at(-1)).toMatchObject({ messageKey: "content.addedLocal", params: { n: "2" } });
  });
});
