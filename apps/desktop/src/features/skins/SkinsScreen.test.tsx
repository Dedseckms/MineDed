// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import "../../i18n";
import { App } from "../../App";
import { api, unwrap } from "../../api/client";
import { setMockFilePicker } from "../../api/dialog";
import i18n from "../../i18n";
import { useNavStore } from "../../stores/navStore";

afterEach(() => {
  cleanup();
  setMockFilePicker(null);
});

for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

beforeAll(async () => {
  const settings = unwrap(await api.settingsGet());
  // Light mode: flat previews instead of WebGL under happy-dom.
  unwrap(await api.settingsSet({ ...settings, language: "en", lightMode: true }));
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

async function openAdd() {
  await act(async () => useNavStore.getState().go("skins"));
  renderApp();
  fireEvent.click(await screen.findByRole("button", { name: "Add skin" }));
  return screen.findByRole("dialog", { name: "Add a skin" });
}

describe("Skins: add skin dialog (mocks)", () => {
  it("adds a PNG chosen on the computer", async () => {
    setMockFilePicker(async (filters) => (filters[0]?.extensions[0] === "png" ? "C:\\Pics\\Hero.png" : null));
    const dialog = await openAdd();
    const add = within(dialog).getByRole("button", { name: "Add to library" });
    expect(add.hasAttribute("disabled")).toBe(true);
    fireEvent.click(within(dialog).getByRole("button", { name: "Choose PNG…" }));
    const name = (await within(dialog).findByLabelText("Name")) as HTMLInputElement;
    expect(name.value).toBe("Hero");
    fireEvent.click(within(dialog).getByRole("radio", { name: "Slim (Alex)" }));
    fireEvent.click(add);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(unwrap(await api.skinLibrary()).find((s) => s.name === "Hero")?.model).toBe("slim");
  });

  it("finds a skin on the web: validation, not found, preview with suggested name and model", async () => {
    const dialog = await openAdd();
    fireEvent.click(within(dialog).getByRole("radio", { name: "From the web" }));
    const input = within(dialog).getByLabelText("Link or nickname");
    const find = within(dialog).getByRole("button", { name: "Find" });

    fireEvent.change(input, { target: { value: "bad nick!" } });
    fireEvent.click(find);
    expect(await within(dialog).findByText(i18n.t("validation.skinSource"))).toBeTruthy();

    fireEvent.change(input, { target: { value: "missing_player" } });
    fireEvent.click(find);
    expect(await within(dialog).findByText(i18n.t("errors.notFound"))).toBeTruthy();

    fireEvent.change(input, { target: { value: "Alexa" } });
    fireEvent.click(find);
    const name = (await within(dialog).findByLabelText("Name")) as HTMLInputElement;
    expect(name.value).toBe("Alexa");
    expect(within(dialog).getByRole("radio", { name: "Slim (Alex)" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.change(name, { target: { value: "Alexa remote" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Add to library" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(unwrap(await api.skinLibrary()).some((s) => s.name === "Alexa remote")).toBe(true);
  });
});


describe("cape import and preview", () => {
  it("shows import errors and rejects insecure URLs before import", async () => {
    setMockFilePicker(async () => "C:\\Pics\\bad.jpg");
    await act(async () => useNavStore.getState().go("skins"));
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "Import cape PNG…" }));
    expect(await screen.findByText(i18n.t("errors.invalidInput"))).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Cape HTTPS URL"), { target: { value: "http://example.com/cape.png" } });
    expect(screen.getByRole("button", { name: "Import cape link" }).hasAttribute("disabled")).toBe(true);
  });

  it("previews a selected library cape without applying, then explicitly applies and removes", async () => {
    await act(async () => useNavStore.getState().go("skins"));
    renderApp();
    const picker = await screen.findByRole("combobox", { name: "Cape" });
    await screen.findByRole("option", { name: "Builder" });
    const account = (screen.getByRole("combobox", { name: "Account" }) as HTMLSelectElement).value;
    unwrap(await api.capeSet(account, null));
    fireEvent.change(picker, { target: { value: "cape-builder" } });
    expect(await screen.findByAltText("Cape preview")).toBeTruthy();
    expect(unwrap(await api.capeList(account)).some((c) => c.active)).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Apply cape" }));
    await waitFor(async () => expect(unwrap(await api.capeList(account)).find((c) => c.active)?.id).toBe("cape-builder"));
    fireEvent.change(picker, { target: { value: "" } });
    fireEvent.click(await screen.findByRole("button", { name: "Remove cape" }));
    await waitFor(async () => expect(unwrap(await api.capeList(account)).some((c) => c.active)).toBe(false));
  });

  it("imports a file and HTTPS link for preview before explicit apply", async () => {
    setMockFilePicker(async () => "C:\\Pics\\Cape.png");
    await act(async () => useNavStore.getState().go("skins"));
    renderApp();
    await screen.findByRole("combobox", { name: "Cape" });
    const account = (screen.getByRole("combobox", { name: "Account" }) as HTMLSelectElement).value;
    unwrap(await api.capeSet(account, null));
    fireEvent.click(screen.getByRole("button", { name: "Import cape PNG…" }));
    await screen.findByRole("option", { name: "Cape.png" });
    expect(unwrap(await api.capeList(account)).some((c) => c.active)).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Apply cape" }));
    await waitFor(async () => expect(unwrap(await api.capeList(account)).find((c) => c.active)?.name).toBe("Cape.png"));
    fireEvent.change(screen.getByLabelText("Cape HTTPS URL"), { target: { value: "https://example.com/remote-cape.png" } });
    fireEvent.click(screen.getByRole("button", { name: "Import cape link" }));
    await screen.findByRole("option", { name: "remote-cape.png" });
    expect(unwrap(await api.capeList(account)).find((c) => c.active)?.name).toBe("Cape.png");
    fireEvent.click(screen.getByRole("button", { name: "Apply cape" }));
    await waitFor(async () => expect(unwrap(await api.capeList(account)).find((c) => c.active)?.name).toBe("remote-cape.png"));
  });
});
