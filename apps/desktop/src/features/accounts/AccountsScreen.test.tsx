// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Commands } from "../../api/client";

const h = vi.hoisted(() => ({ api: null as unknown as Commands }));
vi.mock("../../api/client", async (orig) => ({
  ...await orig<typeof import("../../api/client")>(),
  api: new Proxy({} as Commands, { get: (_target, prop) => Reflect.get(h.api, prop) }),
}));
import { createMockCommands } from "../../mocks";
import { useNavStore } from "../../stores/navStore";
import { handleDeepLink } from "../../hooks/useDeepLinks";
import i18n from "../../i18n";
import { AccountsScreen } from "./AccountsScreen";

for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

let register: ReturnType<typeof vi.fn<Commands["accountYggdrasilRegister"]>>;
let login: ReturnType<typeof vi.fn<Commands["accountYggdrasilLogin"]>>;
beforeEach(async () => {
  await i18n.changeLanguage("en");
  useNavStore.setState({ pendingInvite: null });
  const backend = createMockCommands({}, { noAccounts: true });
  register = vi.fn(backend.accountYggdrasilRegister);
  login = vi.fn(backend.accountYggdrasilLogin);
  h.api = { ...backend, accountYggdrasilRegister: register, accountYggdrasilLogin: login };
});
afterEach(cleanup);

async function mount(registerMode = true) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(<QueryClientProvider client={qc}><AccountsScreen /></QueryClientProvider>);
  await screen.findByText(/Server: /);
  if (registerMode) fireEvent.click(screen.getByRole("radio", { name: i18n.t("accounts.yggdrasil.register") }));
}
function fill(username = "New_Player", password = "secret1234", confirm = password) {
  fireEvent.change(screen.getByLabelText(i18n.t("accounts.nick"), { selector: 'input[autocomplete="username"]' }), { target: { value: username } });
  fireEvent.change(screen.getByLabelText("Password"), { target: { value: password } });
  fireEvent.change(screen.getByLabelText("Repeat password"), { target: { value: confirm } });
}
function submit() { fireEvent.click(screen.getByRole("button", { name: i18n.t("accounts.yggdrasil.register") })); }

describe("open registration", () => {
  it("creates and lists an account without a code, then clears passwords", async () => {
    await mount();
    expect(screen.getByTestId("invite-input").closest("details")?.open).toBe(false);
    fill(); submit();
    await screen.findByText("New_Player");
    expect(register).toHaveBeenCalledExactlyOnceWith(expect.any(String), "", "New_Player", "secret1234");
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Repeat password") as HTMLInputElement).value).toBe("");
  });
  it.each([
    ["bad name", "secret1234", "secret1234", "validation.nick.chars"],
    ["ab", "secret1234", "secret1234", "validation.nick.length"],
    ["New_Player", "short", "short", "validation.password.short"],
    ["New_Player", "123456789", "123456789", "validation.password.short"],
    ["New_Player", "", "", "validation.required"],
    ["New_Player", "secret1234", "different", "validation.password.mismatch"],
  ])("rejects invalid input before requesting registration: %s / %s", async (username, password, confirm, key) => {
    await mount(); fill(username, password, confirm); submit();
    expect(screen.getAllByText(i18n.t(key)).length).toBeGreaterThan(0);
    expect(register).not.toHaveBeenCalled();
  });
  it("preserves invitation deep links and forwards their code", async () => {
    await mount(false);
    act(() => { handleDeepLink("mineded://invite/Existing_123"); });
    await screen.findByLabelText("Repeat password");
    expect((screen.getByTestId("invite-input") as HTMLInputElement).value).toBe("Existing_123");
    expect(screen.getByTestId("invite-input").closest("details")?.open).toBe(true);
    fill(); submit();
    await waitFor(() => expect(register).toHaveBeenCalledWith(expect.any(String), "Existing_123", "New_Player", "secret1234"));
  });
  it("uses the configured custom server for registration", async () => {
    const settings = await h.api.settingsGet();
    if (settings.status !== "ok") throw new Error("missing mock settings");
    const custom = "https://custom.example/api/yggdrasil";
    h.api = { ...h.api, settingsGet: async () => ({ status: "ok", data: { ...settings.data, skinServer: custom } }) };
    await mount(); fill(); submit();
    await waitFor(() => expect(register).toHaveBeenCalledWith(custom, "", "New_Player", "secret1234"));
  });
  it("rejects malformed optional invitation before requesting registration", async () => {
    await mount(); fill();
    fireEvent.click(screen.getByText(i18n.t("accounts.yggdrasil.invite"), { selector: "summary" }));
    fireEvent.change(screen.getByTestId("invite-input"), { target: { value: "bad invite!" } });
    submit();
    expect(screen.getByText(i18n.t("validation.invite"))).toBeTruthy();
    expect(register).not.toHaveBeenCalled();
  });
  it("keeps login credentials and endpoint unchanged", async () => {
    await mount(false);
    fireEvent.change(screen.getByLabelText(i18n.t("accounts.yggdrasil.username")), { target: { value: " Player " } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "existing-password" } });
    fireEvent.click(screen.getByRole("button", { name: i18n.t("accounts.yggdrasil.login") }));
    await waitFor(() => expect(login).toHaveBeenCalledWith(expect.any(String), "Player", "existing-password"));
    expect(register).not.toHaveBeenCalled();
  });
  it("keeps server rejection visible and clears passwords", async () => {
    register.mockResolvedValue({ status: "error", error: { code: "conflict", message: "taken", details: null } });
    await mount(); fill(); submit();
    await screen.findByText(i18n.t("errors.conflict"));
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Repeat password") as HTMLInputElement).value).toBe("");
  });
  it("shows server discovery failures and blocks submission without a server", async () => {
    h.api = createMockCommands({ accountYggdrasilRegister: register, accountYggdrasilServers: async () => ({ status: "error", error: { code: "network", message: "unreachable", details: null } }) });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={qc}><AccountsScreen /></QueryClientProvider>);
    fireEvent.click(screen.getByRole("radio", { name: i18n.t("accounts.yggdrasil.register") }));
    await screen.findByText(i18n.t("errors.network"));
    expect(screen.getByRole("button", { name: i18n.t("accounts.yggdrasil.register") }).hasAttribute("disabled")).toBe(true);
    expect(register).not.toHaveBeenCalled();
  });
});
