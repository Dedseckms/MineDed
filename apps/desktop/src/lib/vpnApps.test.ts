import { describe, expect, it } from "vitest";

import type { Account, VpnApp } from "../bindings";
import { localSkinSrc } from "../features/skins/skinSrc";
import { errorView } from "./errors";
import { validateSkinSource } from "./validation";
import {
  APP_PRESETS,
  addApp,
  appFromPath,
  filterApps,
  hasApp,
  minededAccounts,
  minededMode,
  minededUsage,
  presetApp,
  removeApp,
  sameApps,
  toGb,
} from "./vpn";

const GIB = 1024 ** 3;
const app = (process: string, name = process): VpnApp => ({ name, process, path: null });
const acc = (id: string, kind: Account["kind"], isDefault = false): Account => ({
  id,
  kind,
  name: id,
  uuid: "u",
  server: null,
  skinUrl: null,
  isDefault,
  valid: true,
});

describe("MineDed VPN helpers", () => {
  it("usage share, bytes left and exhaustion", () => {
    expect(minededUsage({ quotaBytes: 100 * GIB, usedBytes: 25 * GIB, resetAt: null, enabled: true })).toEqual({
      used: 0.25,
      leftBytes: 75 * GIB,
      exhausted: false,
    });
    expect(minededUsage({ quotaBytes: 10, usedBytes: 12, resetAt: null, enabled: true })).toMatchObject({
      used: 1,
      leftBytes: 0,
      exhausted: true,
    });
    expect(minededUsage({ quotaBytes: 10, usedBytes: 1, resetAt: null, enabled: false }).exhausted).toBe(true);
    expect(minededUsage({ quotaBytes: 0, usedBytes: 0, resetAt: null, enabled: true }).used).toBe(1);
  });

  it("formats GiB with one decimal", () => {
    expect(toGb(256 * GIB)).toBe(256);
    expect(toGb(1.25 * GIB)).toBe(1.3);
    expect(toGb(-5)).toBe(0);
  });

  it("only own-server accounts, the default one first", () => {
    const list = [acc("a", "offline", true), acc("b", "yggdrasil"), acc("c", "yggdrasil", true)];
    expect(minededAccounts(list).map((a) => a.id)).toEqual(["c", "b"]);
    expect(minededAccounts([acc("a", "offline")])).toEqual([]);
  });

  it("connects in apps mode only when programs are chosen", () => {
    expect(minededMode([])).toBe("launcherAndGame");
    expect(minededMode([app("x.exe")])).toBe("apps");
  });
});

describe("apps list helpers", () => {
  it("presets per OS", () => {
    const tg = APP_PRESETS.find((p) => p.name === "Telegram");
    expect(tg && presetApp(tg, "windows").process).toBe("Telegram.exe");
    expect(tg && presetApp(tg, "linux").process).toBe("telegram-desktop");
    expect(APP_PRESETS.map((p) => p.name)).toEqual([
      "Telegram",
      "Discord",
      "Chrome",
      "Firefox",
      "Edge",
      "Yandex Browser",
      "Opera",
      "Steam",
    ]);
  });

  it("adds without duplicates (case-insensitive), removes, compares", () => {
    let list = addApp([], app("Telegram.exe"));
    list = addApp(list, app("telegram.EXE"));
    list = addApp(list, app("  "));
    expect(list.map((a) => a.process)).toEqual(["Telegram.exe"]);
    expect(hasApp(list, { process: "TELEGRAM.exe" })).toBe(true);
    list = addApp(list, app("steam.exe"));
    expect(removeApp(list, "TELEGRAM.EXE").map((a) => a.process)).toEqual(["steam.exe"]);
    expect(sameApps(list, [app("telegram.exe"), app("steam.exe")])).toBe(true);
    expect(sameApps(list, [app("steam.exe"), app("telegram.exe")])).toBe(false);
  });

  it("app from a picked file", () => {
    expect(appFromPath("C:\\Program Files\\Telegram Desktop\\Telegram.exe")).toEqual({
      name: "Telegram",
      process: "Telegram.exe",
      path: "C:\\Program Files\\Telegram Desktop\\Telegram.exe",
    });
    expect(appFromPath("/usr/bin/firefox")).toMatchObject({ name: "firefox", process: "firefox" });
    expect(appFromPath("C:\\dir\\")).toBeNull();
  });

  it("filters by name or process", () => {
    const list = [app("chrome.exe", "Google Chrome"), app("Code.exe", "Visual Studio Code")];
    expect(filterApps(list, "GOOGLE").map((a) => a.process)).toEqual(["chrome.exe"]);
    expect(filterApps(list, "code.e").map((a) => a.process)).toEqual(["Code.exe"]);
    expect(filterApps(list, " ")).toHaveLength(2);
  });
});

describe("VPN error reasons", () => {
  it("maps uacDeclined and vpnUnavailable to friendly messages", () => {
    expect(errorView({ code: "permissionDenied", message: "", details: { reason: "uacDeclined" } }).messageKey).toBe(
      "errors.reason.uacDeclined",
    );
    expect(errorView({ code: "network", message: "", details: { reason: "vpnUnavailable" } }).messageKey).toBe(
      "errors.reason.vpnUnavailable",
    );
    expect(errorView({ code: "network", message: "", details: { reason: "connection reset" } }).messageKey).toBe(
      "errors.network",
    );
  });
});

describe("skins from the web", () => {
  it("validates a link or a nickname", () => {
    expect(validateSkinSource("")).toBe("validation.required");
    expect(validateSkinSource("Notch")).toBeNull();
    expect(validateSkinSource("https://ely.by/u/notch")).toBeNull();
    expect(validateSkinSource("ftp://x/skin.png")).toBe("validation.url.scheme");
    expect(validateSkinSource("bad nick")).toBe("validation.skinSource");
    expect(validateSkinSource("a".repeat(17))).toBe("validation.skinSource");
  });

  it("previews local files through the asset protocol, URLs as is", () => {
    const toAsset = (p: string) => `asset://localhost/${encodeURIComponent(p)}`;
    expect(localSkinSrc("C:\\cache\\n.png", toAsset)).toBe("asset://localhost/C%3A%5Ccache%5Cn.png");
    expect(localSkinSrc("data:image/png;base64,AA", toAsset)).toBe("data:image/png;base64,AA");
    expect(localSkinSrc("  ", toAsset)).toBeNull();
    expect(
      localSkinSrc("/tmp/x.png", () => {
        throw new Error("no tauri");
      }),
    ).toBeNull();
  });
});
