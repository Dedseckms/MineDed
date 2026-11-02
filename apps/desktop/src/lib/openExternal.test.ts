// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";

const openUrl = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl }));
const { openExternal } = await import("./openExternal");

beforeEach(() => {
  openUrl.mockClear();
  vi.stubGlobal("open", vi.fn());
});
describe("openExternal", () => {
  it("hands http(s) links to the opener plugin", async () => {
    await openExternal("https://modrinth.com/mod/sodium");
    expect(openUrl).toHaveBeenCalledWith("https://modrinth.com/mod/sodium");
    expect(window.open).not.toHaveBeenCalled();
  });
  it("falls back to a new tab when the plugin is unavailable", async () => {
    openUrl.mockRejectedValueOnce(new Error("no ipc"));
    await openExternal("https://www.curseforge.com/minecraft/mc-mods/jei");
    expect(window.open).toHaveBeenCalled();
  });
  it("ignores non-http schemes", async () => {
    await openExternal("javascript:alert(1)");
    expect(openUrl).not.toHaveBeenCalled();
  });
});
