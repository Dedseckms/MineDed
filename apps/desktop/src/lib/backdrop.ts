// Translucent window + built-in backgrounds: flags and readability maths.
import type { Settings } from "../bindings";
import { type Rgb, contrast, mix } from "./color";

/** Built-in backgrounds (rendered by `features/appearance/backgrounds.tsx`). */
export const BACKGROUND_IDS = ["plains", "forest", "nether", "ocean", "snowy", "cave", "desert", "cherry"] as const;
export type BackgroundId = (typeof BACKGROUND_IDS)[number];

/** Known id or `null` (unknown ids from newer/older versions fall back to none). */
export function resolveBackground(id: string | null | undefined): BackgroundId | null {
  return id && (BACKGROUND_IDS as readonly string[]).includes(id) ? (id as BackgroundId) : null;
}

/**
 * Opacity of the theme background laid over the window backdrop (Mica /
 * Acrylic / a background picture) and of the theme surface used by panels
 * (sidebar, header, sections, cards). Keep in sync with `styles.css`.
 */
export const SCRIM_ALPHA = 0.65;
export const PANEL_ALPHA = 0.85;

export interface BackdropFlags {
  /** Transparent `html`/`body`: the OS material shows through. */
  translucent: boolean;
  /** Something sits behind the shell (material or picture): scrim + tinted panels. */
  backdrop: boolean;
  /** Panels are semi-transparent (never in light mode). */
  tintedPanels: boolean;
  background: BackgroundId | null;
}

export function backdropFlags(settings: Settings | undefined, light: boolean): BackdropFlags {
  const translucent = !light && (settings?.translucent ?? false);
  const background = resolveBackground(settings?.background);
  const backdrop = translucent || background !== null;
  return { translucent, backdrop, tintedPanels: backdrop && !light, background };
}

/** Theme colours that matter for readability (keep in sync with `styles.css`). */
export const THEME_PALETTE: Record<"dark" | "light", { bg: Rgb; surface: Rgb; text: Rgb; muted: Rgb }> = {
  dark: {
    bg: { r: 0x0f, g: 0x11, b: 0x15 },
    surface: { r: 0x17, g: 0x1a, b: 0x21 },
    text: { r: 0xe6, g: 0xe8, b: 0xee },
    muted: { r: 0x8a, g: 0x90, b: 0xa0 },
  },
  light: {
    bg: { r: 0xf4, g: 0xf5, b: 0xf8 },
    surface: { r: 0xff, g: 0xff, b: 0xff },
    text: { r: 0x16, g: 0x1a, b: 0x22 },
    muted: { r: 0x5d, g: 0x64, b: 0x75 },
  },
};

const WHITE: Rgb = { r: 255, g: 255, b: 255 };
const BLACK: Rgb = { r: 0, g: 0, b: 0 };

/** `color` at `alpha` over `under` (source-over compositing). */
const over = (color: Rgb, alpha: number, under: Rgb): Rgb => mix(under, color, alpha);

/**
 * Lowest contrast of `fg` on the scrim alone and on a panel over the scrim,
 * over the worst possible backdrop pixel (pure white or pure black).
 */
export function worstCaseContrast(theme: "dark" | "light", fg: "text" | "muted", on: "scrim" | "panel"): number {
  const p = THEME_PALETTE[theme];
  return Math.min(
    ...[WHITE, BLACK].map((backdrop) => {
      const scrim = over(p.bg, SCRIM_ALPHA, backdrop);
      const surface = on === "panel" ? over(p.surface, PANEL_ALPHA, scrim) : scrim;
      return contrast(p[fg], surface);
    }),
  );
}
