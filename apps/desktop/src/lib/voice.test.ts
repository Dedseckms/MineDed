import { describe, expect, it } from "vitest";

import type { ChatMessage, Peer, VoiceSettings } from "../bindings";
import { OVERLAY_WINDOW, isOverlayRoute } from "./overlay";
import {
  applySpeaking,
  captureKey,
  clampGain,
  deviceValue,
  devicesOf,
  fromPercent,
  isSpeaking,
  keyLabel,
  lastLines,
  meterPercent,
  peerVolume,
  peerVolumeKey,
  toPercent,
  voiceStatus,
  withPeerVolume,
} from "./voice";

const peer = (over: Partial<Peer> = {}): Peer => ({
  peerId: "p1",
  name: "Alex",
  uuid: null,
  path: "direct",
  rttMs: 10,
  speaking: false,
  muted: false,
  ...over,
});

const settings = (over: Partial<VoiceSettings> = {}): VoiceSettings => ({
  mode: "vad",
  inputDevice: null,
  outputDevice: null,
  pttKey: null,
  inputGain: 1,
  outputVolume: 1,
  noiseSuppression: true,
  peerVolumes: {},
  ...over,
});

describe("ptt key capture", () => {
  it("keeps keyboard-types codes, Escape cancels, Tab is ignored", () => {
    expect(captureKey("KeyV")).toEqual({ kind: "set", code: "KeyV" });
    expect(captureKey("CapsLock")).toEqual({ kind: "set", code: "CapsLock" });
    expect(captureKey("Escape")).toEqual({ kind: "cancel" });
    expect(captureKey("Tab")).toEqual({ kind: "ignore" });
    expect(captureKey("")).toEqual({ kind: "ignore" });
  });

  it("labels", () => {
    expect(keyLabel("KeyV")).toBe("V");
    expect(keyLabel("Digit4")).toBe("4");
    expect(keyLabel("Numpad0")).toBe("Num 0");
    expect(keyLabel("ControlLeft")).toBe("LCtrl");
    expect(keyLabel("AltRight")).toBe("RAlt");
    expect(keyLabel("F13")).toBe("F13");
  });
});

describe("gains and peer volumes", () => {
  it("clamps and converts", () => {
    expect(clampGain(3)).toBe(2);
    expect(clampGain(-1)).toBe(0);
    expect(clampGain(Number.NaN)).toBe(1);
    expect(toPercent(null)).toBe(100);
    expect(toPercent(0.5)).toBe(50);
    expect(fromPercent(150)).toBe(1.5);
  });

  it("peerVolumes keyed by uuid, else name; 1.0 removes the entry", () => {
    expect(peerVolumeKey(peer({ uuid: "u-1" }))).toBe("u-1");
    expect(peerVolumeKey(peer())).toBe("Alex");
    const s = withPeerVolume(settings(), peer(), 1.5);
    expect(s.peerVolumes).toEqual({ Alex: 1.5 });
    expect(peerVolume(s, peer())).toBe(1.5);
    expect(peerVolume(s, peer({ name: "Other" }))).toBe(1);
    expect(withPeerVolume(s, peer(), 1).peerVolumes).toEqual({});
    expect(peerVolume(undefined, peer())).toBe(1);
  });

  it("devices", () => {
    const list = [
      { id: "a", name: "A", direction: "input" as const, isDefault: true },
      { id: "b", name: "B", direction: "output" as const, isDefault: true },
    ];
    expect(devicesOf(list, "input").map((d) => d.id)).toEqual(["a"]);
    expect(deviceValue(list, "a")).toBe("a");
    expect(deviceValue(list, "gone")).toBe("");
    expect(deviceValue(list, null)).toBe("");
  });
});

describe("status and speaking", () => {
  it("voice status", () => {
    expect(voiceStatus("vad", false, false, null)).toBe("inactive");
    expect(voiceStatus("off", true, false, null)).toBe("off");
    expect(voiceStatus("vad", true, true, null)).toBe("muted");
    expect(voiceStatus("pushToTalk", true, false, null)).toBe("pttNoKey");
    expect(voiceStatus("pushToTalk", true, false, "KeyV")).toBe("pushToTalk");
    expect(voiceStatus("vad", true, false, null)).toBe("vad");
  });

  it("applies speaking events; unchanged events keep the map identity", () => {
    const a = applySpeaking({}, { peerId: "p1", speaking: true, level: 0.5 });
    expect(a).toEqual({ p1: { speaking: true, level: 0.5 } });
    expect(applySpeaking(a, { peerId: "p1", speaking: true, level: 0.501 })).toBe(a);
    expect(applySpeaking(a, { peerId: "p1", speaking: false, level: null }).p1).toEqual({ speaking: false, level: 0 });
    expect(applySpeaking({}, { peerId: "x", speaking: true, level: 7 }).x?.level).toBe(1);
    expect(isSpeaking(peer({ speaking: true }), {})).toBe(true);
    expect(isSpeaking(peer({ speaking: true }), { p1: { speaking: false, level: 0 } })).toBe(false);
  });

  it("meter and chat lines", () => {
    expect(meterPercent(0)).toBe(0);
    expect(meterPercent(0.25)).toBe(50);
    expect(meterPercent(5)).toBe(100);
    expect(meterPercent(Number.NaN)).toBe(0);
    const msgs = Array.from({ length: 6 }, (_, i) => ({ id: `${i}` }) as ChatMessage);
    expect(lastLines(msgs).map((m) => m.id)).toEqual(["2", "3", "4", "5"]);
    expect(lastLines([])).toEqual([]);
  });
});

describe("overlay", () => {
  it("route and window options", () => {
    expect(isOverlayRoute("#/overlay")).toBe(true);
    expect(isOverlayRoute("#/overlay?x=1")).toBe(true);
    expect(isOverlayRoute("")).toBe(false);
    expect(isOverlayRoute("#/overlayx")).toBe(false);
    expect(OVERLAY_WINDOW).toMatchObject({ alwaysOnTop: true, decorations: false, transparent: true, url: "index.html#/overlay" });
  });
});
