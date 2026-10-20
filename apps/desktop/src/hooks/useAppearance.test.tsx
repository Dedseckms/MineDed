// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";

import type { Settings } from "../bindings";
import { qk } from "../api/queries";
import { defaultSettings } from "../mocks/data";
import { motionPrefs, resolveTheme, useApplyAppearance, useMotionPrefs } from "./useAppearance";

describe("appearance helpers", () => {
  it("resolveTheme", () => {
    expect(resolveTheme("dark", false)).toBe("dark");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme({ custom: "creeper" }, false)).toBe("dark");
  });

  it("motionPrefs combines setting, light mode and OS preference", () => {
    const s = defaultSettings();
    expect(motionPrefs(s, false)).toEqual({ reduced: false, light: false });
    expect(motionPrefs(s, true)).toEqual({ reduced: true, light: false });
    expect(motionPrefs({ ...s, reduceMotion: true }, false).reduced).toBe(true);
    expect(motionPrefs({ ...s, lightMode: true }, false)).toEqual({ reduced: true, light: true });
    expect(motionPrefs(undefined, false)).toEqual({ reduced: false, light: false });
  });
});

function withSettings(settings: Settings) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  qc.setQueryData(qk.settings, settings);
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("useApplyAppearance", () => {
  it("applies theme and flags to <html>", async () => {
    const wrapper = withSettings({ ...defaultSettings(), theme: "light", lightMode: true, language: "en" });
    renderHook(() => useApplyAppearance(), { wrapper });
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));
    expect(document.documentElement.dataset.lightMode).toBe("true");
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
    expect(document.documentElement.lang).toBe("en");
  });

  it("useMotionPrefs reads settings", () => {
    const wrapper = withSettings({ ...defaultSettings(), reduceMotion: true });
    const { result } = renderHook(() => useMotionPrefs(), { wrapper });
    expect(result.current).toEqual({ reduced: true, light: false });
  });
});
