import { describe, expect, it } from "vitest";

import { createMockBus } from "./bus";
import { applyDemoSettings, demoOverrides, parseDemoParams, runDemo } from "./demo";
import { createMockCommands } from "./index";
import { defaultSettings } from "./data";

describe("mock bus", () => {
  it("keeps a second subscription of the same callback after the first unsubscribes (StrictMode)", () => {
    const bus = createMockBus();
    const seen: string[] = [];
    const cb = (p: { id: string; lines: string[] }) => seen.push(...p.lines);
    const off = bus.on("instanceLog", cb);
    bus.on("instanceLog", cb);
    off();
    bus.emit("instanceLog", { id: "a", lines: ["x"] });
    expect(seen).toEqual(["x"]);
  });
});

describe("demo mode", () => {
  it("parses params with safe defaults", () => {
    expect(parseDemoParams("?section=play")).toBeNull();
    expect(parseDemoParams("?demo=1")).toEqual({ lang: "ru", theme: "dark", background: "forest" });
    expect(parseDemoParams("?demo&lang=en&theme=light&bg=plains")).toEqual({ lang: "en", theme: "light", background: "plains" });
    expect(parseDemoParams("?demo&lang=de&bg=nope")).toEqual({ lang: "ru", theme: "dark", background: "forest" });
  });

  it("applies appearance to settings", () => {
    const s = applyDemoSettings(defaultSettings(), { lang: "en", theme: "dark", background: "cherry" });
    expect(s).toMatchObject({ language: "en", theme: "dark", background: "cherry", translucent: false });
  });

  it("seeds vpn, room, voice and a release version", async () => {
    const bus = createMockBus();
    const api = createMockCommands(demoOverrides("1.0.5"), { bus, tickMs: 1 });
    await runDemo(api, bus, { lang: "ru", theme: "dark", background: "forest" });
    expect(await api.appVersion()).toBe("1.0.5");
    expect(await api.updateCheck()).toEqual({ status: "ok", data: null });
    const vpn = await api.vpnState();
    expect(vpn.status === "ok" && vpn.data.running).toBe(true);
    const room = await api.roomState();
    expect(room.status === "ok" && room.data?.role).toBe("host");
    const voice = await api.voiceStatus();
    expect(voice.status === "ok" && voice.data.active).toBe(true);
    const accounts = await api.accountList();
    expect(accounts.status === "ok" && accounts.data[0]?.skinUrl).toBeTruthy();
    await api.roomLeave();
    await api.voiceStop();
  });
});
