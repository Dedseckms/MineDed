import { useEffect, useSyncExternalStore } from "react";

import type { Settings, Theme } from "../bindings";
import { useSettings } from "../api/queries";
import i18n from "../i18n";

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

/** Applies theme, language and performance flags from settings to the document. */
export function useApplyAppearance(): void {
  const { data: settings } = useSettings();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const prefs = useMotionPrefs();
  const theme = resolveTheme(settings?.theme ?? "system", prefersDark);
  const language = settings?.language;

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme;
    root.dataset.lightMode = String(prefs.light);
    root.dataset.reduceMotion = String(prefs.reduced);
    root.style.colorScheme = theme;
  }, [theme, prefs.light, prefs.reduced]);

  useEffect(() => {
    if (language && i18n.language !== language) void i18n.changeLanguage(language);
    if (language) document.documentElement.lang = language;
  }, [language]);
}
