// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { commands, RoomState, SocialContent } from "../../bindings";
import { seedInstances } from "../../mocks/data";
import { SOCIAL_PEER } from "../../mocks/social";
import i18n from "../../i18n";
import { SharePack, ReceivePack } from "./PackShare";

for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

type Commands = typeof commands;
const mocks = vi.hoisted(() => ({
  roomState: vi.fn<Commands["roomState"]>(),
  roomJoin: vi.fn<Commands["roomJoin"]>(),
  roomDownloadHostPack: vi.fn<Commands["roomDownloadHostPack"]>(),
  instanceList: vi.fn<Commands["instanceList"]>(),
  socialSharePack: vi.fn<Commands["socialSharePack"]>(),
}));
vi.mock("../../api/client", async (original) => ({
  ...await original<typeof import("../../api/client")>(), api: mocks,
}));
const pack: Extract<SocialContent, { type: "packShare" }> = {
  type: "packShare", name: "Pack", mc: "1.21.1", loader: "fabric", mods: 10,
  code: "PACK-1234", passwordProtected: false,
};
const room: RoomState = {
  roomId: "room", code: "PACK-1234", role: "guest", kind: "game", authMode: "open", sig: null,
  peers: [], expiresAt: "2026-10-30T12:00:00Z", link: "https://example.com", passwordProtected: false, localAddr: null,
};
function mount(child: React.ReactNode) {
  return render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })}>{child}</QueryClientProvider>);
}
beforeEach(async () => {
  vi.resetAllMocks();
  await i18n.changeLanguage("en");
  mocks.roomState.mockResolvedValue({ status: "ok", data: null });
  mocks.roomJoin.mockResolvedValue({ status: "ok", data: room });
  mocks.roomDownloadHostPack.mockResolvedValue({ status: "ok", data: "download-task" });
  mocks.instanceList.mockResolvedValue({ status: "ok", data: seedInstances() });
});
afterEach(cleanup);
it("shares the explicitly selected local instance with the native command", async () => {
  mocks.socialSharePack.mockResolvedValue({ status: "ok", data: { id: "1", channel: "dm", author: SOCIAL_PEER, content: pack, createdAt: "2026-10-30T12:00:00Z", editedAt: null, deleted: false } });
  mount(<SharePack channel="dm" disabled={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Share modpack" }));
  const dialog = await screen.findByRole("dialog");
  await within(dialog).findByRole("option", { name: /Fabric 1.21.1/ });
  fireEvent.change(within(dialog).getByRole("combobox"), { target: { value: "inst-fabric" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Share selected instance" }));
  await waitFor(() => expect(mocks.socialSharePack).toHaveBeenCalledWith("dm", "inst-fabric"));
});
it("keeps legacy cards without a code unavailable", () => {
  mount(<ReceivePack pack={{ ...pack, code: null }} offline={false} />);
  expect(screen.getByRole("button", { name: "Download modpack" }).hasAttribute("disabled")).toBe(true);
  expect(mocks.roomJoin).not.toHaveBeenCalled();
});
it("joins before starting the existing host-pack download task", async () => {
  mount(<ReceivePack pack={pack} offline={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Download modpack" }));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Download modpack" }));
  await waitFor(() => expect(mocks.roomDownloadHostPack).toHaveBeenCalledOnce());
  expect(mocks.roomJoin).toHaveBeenCalledWith("PACK-1234", null, null);
  expect(mocks.roomJoin.mock.invocationCallOrder[0]).toBeLessThan(mocks.roomDownloadHostPack.mock.invocationCallOrder[0]!);
});
it("blocks an unrelated active room without joining or leaving it", async () => {
  mocks.roomState.mockResolvedValue({ status: "ok", data: { ...room, code: "OTHER-1234" } });
  mount(<ReceivePack pack={pack} offline={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Download modpack" }));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Download modpack" }));
  expect(await screen.findByText(i18n.t("errors.reason.packRoomBusy"))).toBeTruthy();
  expect(mocks.roomJoin).not.toHaveBeenCalled();
  expect(mocks.roomDownloadHostPack).not.toHaveBeenCalled();
});
it("asks for a protected pack password and clears it after joining", async () => {
  mount(<ReceivePack pack={{ ...pack, passwordProtected: true }} offline={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Download modpack" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("Room password"), { target: { value: "secret" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Download modpack" }));
  await waitFor(() => expect(mocks.roomJoin).toHaveBeenCalledWith("PACK-1234", null, "secret"));
  await waitFor(() => expect((within(dialog).getByLabelText("Room password") as HTMLInputElement).value).toBe(""));
});

it("reports task completion after the instance list is refreshed", async () => {
  mount(<ReceivePack pack={pack} offline={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Download modpack" }));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Download modpack" }));
  await waitFor(() => expect(mocks.roomDownloadHostPack).toHaveBeenCalledOnce());
  await act(async () => mocks.roomDownloadHostPack.mock.calls[0]?.[0].onmessage({ type: "finished", id: "download-task", error: null }));
  expect(await within(screen.getByRole("dialog")).findByRole("status")).toHaveProperty("textContent", "Modpack imported");
  expect(mocks.instanceList).toHaveBeenCalled();
});
it("shows a failed transfer error and lets the user retry", async () => {
  mount(<ReceivePack pack={pack} offline={false} />);
  fireEvent.click(screen.getByRole("button", { name: "Download modpack" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "Download modpack" }));
  await waitFor(() => expect(mocks.roomDownloadHostPack).toHaveBeenCalledOnce());
  await act(async () => mocks.roomDownloadHostPack.mock.calls[0]?.[0].onmessage({ type: "finished", id: "download-task", error: { code: "network", message: "offline", details: null } }));
  expect(await within(dialog).findByText(i18n.t("errors.network"))).toBeTruthy();
  expect(within(dialog).getByRole("button", { name: "Download modpack" }).hasAttribute("disabled")).toBe(false);
});
