// Pure helpers for voice chat (no React, no backend calls).
import type { AudioDevice, ChatMessage, Peer, VoiceMode, VoiceSettings, VoiceSpeakingPayload } from "../bindings";

/**
 * `peerId` of `voice://speaking` events about the local microphone (own
 * speaking indicator and the mic-test level meter). Agreed with voice; see the
 * report's CONTRACT REQUEST.
 */
export const SELF_PEER_ID = "self";

export const GAIN_MIN = 0;
export const GAIN_MAX = 2;
/** Chat lines shown in the compact overlay. */
export const OVERLAY_CHAT_LINES = 4;

/** Keys that cannot be bound as push-to-talk (used for UI control). */
const RESERVED = new Set(["Escape", "Tab", "Unidentified", ""]);

export type KeyCapture = { kind: "cancel" } | { kind: "set"; code: string } | { kind: "ignore" };

/**
 * Maps a `KeyboardEvent.code` (W3C UI Events naming, identical to Rust
 * `keyboard_types::Code`, e.g. `KeyV`, `CapsLock`, `F13`) to a capture result.
 * Escape cancels; Tab and unidentified keys are ignored.
 */
export function captureKey(code: string): KeyCapture {
  if (code === "Escape") return { kind: "cancel" };
  if (RESERVED.has(code)) return { kind: "ignore" };
  return { kind: "set", code };
}

/** Short human label of a key code: `KeyV` → `V`, `Digit1` → `1`, `Numpad5` → `Num 5`. */
export function keyLabel(code: string): string {
  let m = /^Key([A-Z])$/.exec(code);
  if (m?.[1]) return m[1];
  m = /^Digit(\d)$/.exec(code);
  if (m?.[1]) return m[1];
  m = /^Numpad(.+)$/.exec(code);
  if (m?.[1]) return `Num ${m[1]}`;
  m = /^(Control|Shift|Alt|Meta)(Left|Right)$/.exec(code);
  if (m?.[1] && m[2]) return `${m[2] === "Left" ? "L" : "R"}${m[1] === "Control" ? "Ctrl" : m[1]}`;
  return code;
}

export function clampGain(v: number): number {
  if (!Number.isFinite(v)) return 1;
  return Math.min(GAIN_MAX, Math.max(GAIN_MIN, v));
}

/** `1.0` → `100`. */
export const toPercent = (v: number | null | undefined): number => Math.round(clampGain(v ?? 1) * 100);
export const fromPercent = (p: number): number => clampGain(p / 100);

/** Key of `VoiceSettings.peerVolumes`: profile UUID, or display name for offline players. */
export const peerVolumeKey = (peer: Pick<Peer, "uuid" | "name">): string => peer.uuid ?? peer.name;

export function peerVolume(settings: VoiceSettings | undefined, peer: Pick<Peer, "uuid" | "name">): number {
  return clampGain(settings?.peerVolumes[peerVolumeKey(peer)] ?? 1);
}

/** Returns settings with an updated per-peer volume (1.0 = default, removed from the map). */
export function withPeerVolume(settings: VoiceSettings, peer: Pick<Peer, "uuid" | "name">, volume: number): VoiceSettings {
  const key = peerVolumeKey(peer);
  const peerVolumes = { ...settings.peerVolumes };
  const v = clampGain(volume);
  if (v === 1) delete peerVolumes[key];
  else peerVolumes[key] = v;
  return { ...settings, peerVolumes };
}

export const devicesOf = (list: readonly AudioDevice[] | undefined, dir: AudioDevice["direction"]): AudioDevice[] =>
  (list ?? []).filter((d) => d.direction === dir);

/** Selected device id if it is still present, otherwise `null` (= system default). */
export function deviceValue(list: readonly AudioDevice[], id: string | null): string {
  return id !== null && list.some((d) => d.id === id) ? id : "";
}

export type VoiceStatus = "inactive" | "off" | "muted" | "pushToTalk" | "vad" | "pttNoKey";

/** What the local microphone currently does, for the status badge. */
export function voiceStatus(mode: VoiceMode | undefined, active: boolean, selfMuted: boolean, pttKey: string | null): VoiceStatus {
  if (!active) return "inactive";
  if (mode === undefined || mode === "off") return "off";
  if (selfMuted) return "muted";
  if (mode === "pushToTalk" && !pttKey) return "pttNoKey";
  return mode;
}

export interface SpeakingInfo {
  speaking: boolean;
  level: number;
}

/** Applies a `voice://speaking` event to a per-peer map (returns the same map if nothing changed). */
export function applySpeaking(
  map: Readonly<Record<string, SpeakingInfo>>,
  ev: VoiceSpeakingPayload,
): Record<string, SpeakingInfo> {
  const level = Math.min(1, Math.max(0, ev.level ?? (ev.speaking ? 1 : 0)));
  const prev = map[ev.peerId];
  if (prev && prev.speaking === ev.speaking && Math.abs(prev.level - level) < 0.01) return map as Record<string, SpeakingInfo>;
  return { ...map, [ev.peerId]: { speaking: ev.speaking, level } };
}

/** Whether a peer is speaking: live event wins over the (slower) room state flag. */
export function isSpeaking(peer: Peer, map: Readonly<Record<string, SpeakingInfo>>): boolean {
  return map[peer.peerId]?.speaking ?? peer.speaking;
}

export const lastLines = (messages: readonly ChatMessage[], n = OVERLAY_CHAT_LINES): ChatMessage[] =>
  messages.slice(Math.max(0, messages.length - n));

/** Meter bar width (0–100 %) from a 0..1 level, with a perceptual curve. */
export function meterPercent(level: number): number {
  const l = Math.min(1, Math.max(0, Number.isFinite(level) ? level : 0));
  return Math.round(Math.sqrt(l) * 100);
}
