// Soft UI sound cues (Web Audio, generated: no audio files in the bundle).

export type Cue = "pttOn" | "pttOff";

/** Tone of each cue: rising blip on press, falling on release. */
export const CUES: Record<Cue, { from: number; to: number; ms: number }> = {
  pttOn: { from: 620, to: 880, ms: 70 },
  pttOff: { from: 760, to: 480, ms: 80 },
};

/** Peak gain: quiet enough not to cover voices. */
export const CUE_GAIN = 0.06;

/**
 * Cue for a change of the local "transmitting" flag in push-to-talk mode
 * (`voice://speaking` for `self` follows the key: `open && !selfMuted`).
 */
export function pttCue(prev: boolean, next: boolean): Cue | null {
  if (prev === next) return null;
  return next ? "pttOn" : "pttOff";
}

let ctx: AudioContext | null = null;

/** Plays a cue; silently does nothing where Web Audio is unavailable. */
export function playCue(cue: Cue): void {
  const Ctor = typeof window !== "undefined" ? window.AudioContext : undefined;
  if (!Ctor) return;
  try {
    ctx ??= new Ctor();
    const { from, to, ms } = CUES[cue];
    const now = ctx.currentTime;
    const end = now + ms / 1000;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.setValueAtTime(from, now);
    osc.frequency.exponentialRampToValueAtTime(to, end);
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(CUE_GAIN, now + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, end);
    osc.connect(gain).connect(ctx.destination);
    osc.start(now);
    osc.stop(end + 0.02);
  } catch {
    // Audio output is optional feedback.
  }
}
