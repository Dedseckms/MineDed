import { useEffect, useSyncExternalStore } from "react";

import type { Settings, Theme } from "../bindings";
import { useSettings } from "../api/queries";
import i18n from "../i18n";
import { type BackdropFlags, backdropFlags } from "../lib/backdrop";
import { accentTokens } from "../lib/color";
import { type MotionPreset, type PresetName, motionPreset } from "../lib/motion";

export type ResolvedTheme = "dark" | "light";

/** `system` follows the OS; unknown custom themes fall back to dark. */
export function resolveTheme(theme: Theme, prefersDark: boolean): ResolvedTheme {
  if (theme === "dark" || theme === "light") return theme;
  if (theme === "system") return prefersDark ? "dark" : "light";
  return "dark";
}

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (cb) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      const mql = window.matchMedia(query);
      mql.addEventListener("change", cb);
      return () => mql.removeEventListener("change", cb);
    },
    () => (typeof window !== "undefined" && window.matchMedia ? window.matchMedia(query).matches : false),
    () => false,
  );
}

export interface MotionPrefs {
  /** Animations off: user setting, light mode or OS `prefers-reduced-motion`. */
  reduced: boolean;
  /** Light mode: no blur / backdrop-filter. */
  light: boolean;
}

export function motionPrefs(settings: Settings | undefined, osReduced: boolean): MotionPrefs {
  const light = settings?.lightMode ?? false;
  return { light, reduced: osReduced || light || (settings?.reduceMotion ?? false) };
}

export function useMotionPrefs(): MotionPrefs {
  const { data } = useSettings();
  const osReduced = useMediaQuery("(prefers-reduced-motion: reduce)");
  return motionPrefs(data, osReduced);
}

/** Animation props for a shared preset; instant when motion is reduced. */
export function useMotionPreset(name: PresetName): MotionPreset {
  return motionPreset(name, useMotionPrefs().reduced);
}

const ACCENT_VARS = {
  accent: "--color-accent",
  accentStrong: "--color-accent-strong",
  accentFg: "--color-accent-fg",
} as const;

/** Sets (or clears, for `null`/invalid) the accent CSS variables on `root`. */
export function applyAccent(root: HTMLElement, accent: string | null | undefined, theme: ResolvedTheme): void {
  const tokens = accent ? accentTokens(accent, theme) : null;
  for (const [k, cssVar] of Object.entries(ACCENT_VARS) as Array<[keyof typeof ACCENT_VARS, string]>) {
    if (tokens) root.style.setProperty(cssVar, tokens[k]);
    else root.style.removeProperty(cssVar);
  }
}

/**
 * Translucency flags on `root` (`data-translucent`, `data-backdrop`,
 * `data-tinted`); `styles.css` turns them into transparent body, scrim and
 * semi-transparent panels.
 */
export function applyBackdrop(root: HTMLElement, flags: Pick<BackdropFlags, "translucent" | "backdrop" | "tintedPanels">): void {
  root.dataset.translucent = String(flags.translucent);
  root.dataset.backdrop = String(flags.backdrop);
  root.dataset.tinted = String(flags.tintedPanels);
}

/** Applies theme, accent, language and performance flags from settings to the document. */
export function useApplyAppearance(): void {
  const { data: settings } = useSettings();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const prefs = useMotionPrefs();
  const theme = resolveTheme(settings?.theme ?? "system", prefersDark);
  const language = settings?.language;
  const accent = settings?.accentColor ?? null;

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme;
    root.dataset.lightMode = String(prefs.light);
    root.dataset.reduceMotion = String(prefs.reduced);
    root.style.colorScheme = theme;
  }, [theme, prefs.light, prefs.reduced]);

  useEffect(() => {
    applyAccent(document.documentElement, accent, theme);
  }, [accent, theme]);

  const { translucent, backdrop, tintedPanels } = backdropFlags(settings, prefs.light);
  useEffect(() => {
    applyBackdrop(document.documentElement, { translucent, backdrop, tintedPanels });
  }, [translucent, backdrop, tintedPanels]);

  useEffect(() => {
    if (language && i18n.language !== language) void i18n.changeLanguage(language);
    if (language) document.documentElement.lang = language;
  }, [language]);
}
