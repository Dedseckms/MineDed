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
