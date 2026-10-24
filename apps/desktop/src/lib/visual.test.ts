import { describe, expect, it, vi } from "vitest";

import type { ContentVersion, Instance, Settings } from "../bindings";
import { ApiError } from "../api/client";
import { BACKGROUND_IDS, backdropFlags, resolveBackground, worstCaseContrast } from "./backdrop";
import { BORDERLESS_MODS, findBorderlessMod, installedBorderless } from "./borderless";
import { errorView } from "./errors";
import { authModeFor, effectiveAuthMode } from "./rooms";
import { pttCue } from "./sound";
import en from "../i18n/en.json";

const settings = (patch: Partial<Settings>) => ({ translucent: false, background: null, ...patch }) as Settings;

describe("backdrop", () => {
  it("translucency is off in light mode; backgrounds keep panels opaque there", () => {
    expect(backdropFlags(settings({ translucent: true }), false)).toMatchObject({ translucent: true, backdrop: true, tintedPanels: true });
    expect(backdropFlags(settings({ translucent: true }), true)).toMatchObject({ translucent: false, backdrop: false, tintedPanels: false });
    expect(backdropFlags(settings({ background: "nether" }), true)).toMatchObject({ backdrop: true, tintedPanels: false, background: "nether" });
    expect(backdropFlags(settings({}), false)).toMatchObject({ backdrop: false, background: null });
  });

  it("ignores unknown background ids", () => {
    expect(resolveBackground("ocean")).toBe("ocean");
    expect(resolveBackground("mojang-panorama")).toBeNull();
    expect(resolveBackground(null)).toBeNull();
    expect(BACKGROUND_IDS.length).toBeGreaterThanOrEqual(6);
  });

  it("keeps text WCAG AA over any backdrop pixel", () => {
    for (const theme of ["dark", "light"] as const) {
      expect(worstCaseContrast(theme, "text", "scrim")).toBeGreaterThanOrEqual(4.5);
      expect(worstCaseContrast(theme, "text", "panel")).toBeGreaterThanOrEqual(4.5);
      expect(worstCaseContrast(theme, "muted", "panel")).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("has a name for every background", () => {
    for (const id of BACKGROUND_IDS) expect(typeof en.settings.backgrounds[id]).toBe("string");
  });
});

describe("errorView", () => {
  it("maps known reasons and keeps details first-ordered", () => {
    const v = errorView(
      new ApiError({ code: "invalidInput", message: "mismatch", details: { expected: "open", reason: "authModeMismatch", step: "create" } }),
    );
    expect(v.messageKey).toBe("errors.reason.authModeMismatch");
    expect(v.details.map(([k]) => k)).toEqual(["reason", "step", "expected"]);
    expect(v.copyText).toContain("code: invalidInput");
    expect(v.copyText).toContain("reason: authModeMismatch");
  });

  it("falls back to the code, detects useProxy and rate limits", () => {
    expect(errorView({ code: "network", message: "", details: { hint: "useProxy" } })).toMatchObject({
      messageKey: "errors.network",
      suggestProxy: true,
    });
    expect(errorView({ code: "rateLimited", message: "", details: { retryAfter: "30" } })).toMatchObject({
      messageKey: "errors.reason.rateLimitedRetry",
      params: { seconds: "30" },
    });
    expect(errorView({ code: "network", message: "", details: { reason: "relayUnreachable" } }).messageKey).toBe("errors.reason.relayUnreachable");
    expect(errorView(new Error("boom")).code).toBe("internal");
  });
});

describe("room auth mode", () => {
  it("follows the default account and disables the incompatible mode", () => {
    expect(authModeFor({ kind: "yggdrasil" })).toEqual({ suggested: "yggdrasil", disabled: "open" });
    expect(authModeFor({ kind: "offline" })).toEqual({ suggested: "open", disabled: "yggdrasil" });
    expect(authModeFor(null)).toEqual({ suggested: "open", disabled: null });
    const off = authModeFor({ kind: "offline" });
    expect(effectiveAuthMode(off, "yggdrasil")).toBe("open");
    expect(effectiveAuthMode(authModeFor(null), "yggdrasil")).toBe("yggdrasil");
  });
});

describe("ptt cue", () => {
  it("plays on edges only", () => {
    expect(pttCue(false, true)).toBe("pttOn");
    expect(pttCue(true, false)).toBe("pttOff");
    expect(pttCue(true, true)).toBeNull();
  });
});

describe("borderless helper", () => {
  const inst = (kind: Instance["loader"]["kind"]) => ({ mc: "1.21.1", loader: { kind, version: null } });
  const version = (projectId: string) => ({ projectId, versionId: `${projectId}-v1` }) as ContentVersion;

  it("has candidates for every modded loader and none for vanilla", () => {
    for (const ids of Object.values(BORDERLESS_MODS)) expect(ids.length).toBeGreaterThan(0);
  });

  it("picks the first candidate with a compatible version", async () => {
    const list = vi.fn(async (id: string) => (id === "kYq5qkSL" ? [version(id)] : []));
    expect(await findBorderlessMod(inst("fabric"), list)).toEqual({ source: "modrinth", projectId: "kYq5qkSL", versionId: "kYq5qkSL-v1" });
    expect(list).toHaveBeenCalledWith("UIexcMP1", { mc: "1.21.1", loader: "fabric" });
    expect(await findBorderlessMod(inst("vanilla"), list)).toBeNull();
  });

  it("skips failing candidates but throws when all fail", async () => {
    const flaky = vi.fn(async (id: string) => {
      if (id === "CnalMsa0") throw new Error("net");
      return [version(id)];
    });
    expect((await findBorderlessMod(inst("forge"), flaky))?.projectId).toBe("kYq5qkSL");
    await expect(findBorderlessMod(inst("neoForge"), async () => Promise.reject(new Error("down")))).rejects.toThrow("down");
  });

  it("detects an installed borderless mod", () => {
    const row = { source: "modrinth", projectId: "CnalMsa0" } as const;
    expect(installedBorderless({ loader: { kind: "forge", version: null } }, [row as never])).toBe(row);
    expect(installedBorderless({ loader: { kind: "fabric", version: null } }, [row as never])).toBeNull();
  });
});
