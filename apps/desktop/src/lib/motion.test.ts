import { describe, expect, it } from "vitest";

import { INSTANT, type PresetName, motionPreset } from "./motion";

const NAMES: PresetName[] = ["screen", "backdrop", "dialog", "popover", "toast", "item"];

describe("motion presets", () => {
  it("animate in and out by default", () => {
    for (const name of NAMES) {
      const p = motionPreset(name, false);
      expect(p.initial, name).not.toBe(false);
      expect(p.animate.opacity, name).toBe(1);
      expect(p.exit.opacity, name).toBe(0);
      expect((p.transition as { duration?: number }).duration, name).toBeGreaterThan(0);
    }
  });

  it("are instant when motion is reduced (also light mode)", () => {
    for (const name of NAMES) {
      const p = motionPreset(name, true);
      expect(p.initial, name).toBe(false);
      expect(p.transition, name).toEqual(INSTANT);
      // No movement on exit either.
      expect(Object.keys(p.exit), name).toEqual(["opacity"]);
    }
  });
});
