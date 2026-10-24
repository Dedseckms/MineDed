import type { TargetAndTransition, Transition } from "motion/react";

/** Shared enter/exit animations so every screen, dialog and list feels the same. */
export type PresetName = "screen" | "backdrop" | "dialog" | "popover" | "toast" | "item";

export interface MotionPreset {
  initial: TargetAndTransition | false;
  animate: TargetAndTransition;
  exit: TargetAndTransition;
  transition: Transition;
}

const EASE = [0.2, 0.8, 0.2, 1] as const;

const PRESETS: Record<PresetName, MotionPreset> = {
  screen: {
    initial: { opacity: 0, y: 8 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0, y: -4 },
    transition: { duration: 0.18, ease: EASE },
  },
  backdrop: {
    initial: { opacity: 0 },
    animate: { opacity: 1 },
    exit: { opacity: 0 },
    transition: { duration: 0.15 },
  },
  dialog: {
    initial: { opacity: 0, scale: 0.97, y: 8 },
    animate: { opacity: 1, scale: 1, y: 0 },
    exit: { opacity: 0, scale: 0.97, y: 8 },
    transition: { duration: 0.18, ease: EASE },
  },
  popover: {
    initial: { opacity: 0, y: -6 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0, y: -6 },
    transition: { duration: 0.15, ease: EASE },
  },
  toast: {
    initial: { opacity: 0, y: 12 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0, x: 24 },
    transition: { duration: 0.18, ease: EASE },
  },
  item: {
    initial: { opacity: 0, y: 6 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0 },
    transition: { duration: 0.2, ease: EASE },
  },
};

export const INSTANT: Transition = { duration: 0 };

/**
 * Props for a `motion.*` element. With `reduced` (user setting, light mode or
 * OS preference) nothing animates: elements appear and disappear instantly.
 */
export function motionPreset(name: PresetName, reduced: boolean): MotionPreset {
  const p = PRESETS[name];
  if (!reduced) return p;
  return { initial: false, animate: p.animate, exit: { opacity: 0 }, transition: INSTANT };
}
