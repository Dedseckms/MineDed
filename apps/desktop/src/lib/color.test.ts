import { describe, expect, it } from "vitest";

import { ACCENT_PRESETS, MIN_ACCENT_CONTRAST, THEME_SURFACE, accentTokens, contrast, isHexColor, parseHex, toHex } from "./color";

describe("color", () => {
  it("parses and formats #rrggbb", () => {
    expect(parseHex("#4ade80")).toEqual({ r: 0x4a, g: 0xde, b: 0x80 });
    expect(parseHex("#ABCDEF")).toEqual({ r: 0xab, g: 0xcd, b: 0xef });
    expect(parseHex("#abc")).toBeNull();
    expect(parseHex("red")).toBeNull();
    expect(toHex({ r: 1, g: 255, b: 300 })).toBe("#01ffff");
    expect(isHexColor("#000000")).toBe(true);
    expect(isHexColor("000000")).toBe(false);
  });

  it("computes WCAG contrast", () => {
    const black = { r: 0, g: 0, b: 0 };
    const white = { r: 255, g: 255, b: 255 };
    expect(contrast(black, white)).toBeCloseTo(21, 5);
    expect(contrast(white, white)).toBeCloseTo(1, 5);
  });

  it("returns null for invalid input", () => {
    expect(accentTokens("nope", "dark")).toBeNull();
  });

  const samples = [...ACCENT_PRESETS, "#000000", "#ffffff", "#ffff00", "#0000ff", "#123456", "#7f7f7f"];
  for (const theme of ["dark", "light"] as const) {
    it(`keeps every accent readable in the ${theme} theme`, () => {
      for (const hex of samples) {
        const tokens = accentTokens(hex, theme);
        expect(tokens, hex).not.toBeNull();
        if (!tokens) continue;
        const accent = parseHex(tokens.accent)!;
        const strong = parseHex(tokens.accentStrong)!;
        const fg = parseHex(tokens.accentFg)!;
        expect(contrast(accent, THEME_SURFACE[theme]), `${hex} on surface`).toBeGreaterThanOrEqual(MIN_ACCENT_CONTRAST);
        // Button labels (accent-fg on accent / hover shade) meet WCAG AA for UI text.
        expect(contrast(fg, accent), `${hex} label`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(fg, strong), `${hex} hover label`).toBeGreaterThanOrEqual(3);
      }
    });
  }

  it("leaves an already readable accent untouched", () => {
    expect(accentTokens("#4ade80", "dark")?.accent).toBe("#4ade80");
    expect(accentTokens("#4ade80", "dark")?.accentFg).toBe("#000000");
    expect(accentTokens("#1d4ed8", "light")?.accentFg).toBe("#ffffff");
  });
});
