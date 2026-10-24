/** Accent colour helpers: parse `#rrggbb`, WCAG contrast, derived CSS tokens. */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

const HEX = /^#([0-9a-f]{6})$/i;

export function isHexColor(value: string): boolean {
  return HEX.test(value);
}

export function parseHex(value: string): Rgb | null {
  const m = HEX.exec(value.trim());
  if (!m || !m[1]) return null;
  const n = Number.parseInt(m[1], 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

export function toHex({ r, g, b }: Rgb): string {
  const h = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}`;
}

/** Linear interpolation from `a` to `b` (t = 0..1). */
export function mix(a: Rgb, b: Rgb, t: number): Rgb {
  return { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t };
}

/** WCAG 2.x relative luminance. */
export function luminance({ r, g, b }: Rgb): number {
  const ch = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

export function contrast(a: Rgb, b: Rgb): number {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const WHITE: Rgb = { r: 255, g: 255, b: 255 };
const BLACK: Rgb = { r: 0, g: 0, b: 0 };

/** Surface colours of the built-in themes (keep in sync with styles.css). */
export const THEME_SURFACE: Record<"dark" | "light", Rgb> = {
  dark: { r: 0x17, g: 0x1a, b: 0x21 },
  light: { r: 0xff, g: 0xff, b: 0xff },
};

/** Minimum contrast of the accent against the surface (WCAG non-text / large text). */
export const MIN_ACCENT_CONTRAST = 3;

/** Minimum contrast of the button label (accent-fg) on the accent (WCAG AA). */
export const MIN_LABEL_CONTRAST = 4.5;

export interface AccentTokens {
  accent: string;
  accentStrong: string;
  accentFg: string;
}

/**
 * Derives accent tokens for a theme. The accent is nudged towards white (dark
 * theme) or black (light theme) until it reaches 3:1 on the surface and its
 * label colour (black or white, whichever reads better) reaches 4.5:1.
 */
export function accentTokens(hex: string, theme: "dark" | "light"): AccentTokens | null {
  const base = parseHex(hex);
  if (!base) return null;
  const surface = THEME_SURFACE[theme];
  const towards = theme === "dark" ? WHITE : BLACK;
  const bestFg = (c: Rgb) => (contrast(BLACK, c) >= contrast(WHITE, c) ? BLACK : WHITE);
  const readable = (c: Rgb) =>
    contrast(c, surface) >= MIN_ACCENT_CONTRAST && contrast(bestFg(c), c) >= MIN_LABEL_CONTRAST;
  let accent = base;
  for (let t = 0.05; !readable(accent) && t <= 1.0001; t += 0.05) {
    accent = mix(base, towards, Math.min(1, t));
  }
  const fg = bestFg(accent);
  // Hover shade: slightly darker in both themes (matches the default palette).
  const strong = mix(accent, BLACK, 0.15);
  return { accent: toHex(accent), accentStrong: toHex(strong), accentFg: toHex(fg) };
}

/** Swatches offered in Settings (null = theme default). */
export const ACCENT_PRESETS = ["#4ade80", "#38bdf8", "#818cf8", "#c084fc", "#f472b6", "#f87171", "#fb923c", "#facc15"] as const;
