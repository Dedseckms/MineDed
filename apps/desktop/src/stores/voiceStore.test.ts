import { beforeEach, describe, expect, it } from "vitest";

import type { VoiceStatus } from "../bindings";
import { SELF_PEER_ID } from "../lib/voice";
import { useVoiceStore } from "./voiceStore";

beforeEach(() => useVoiceStore.getState().reset());

const status = (over: Partial<VoiceStatus> = {}): VoiceStatus => ({
  active: true,
  selfMuted: false,
  mode: "vad",
  micTest: false,
  ...over,
});

describe("voice store", () => {
  it("tracks local peer mutes and indicators; resets them when the room changes", () => {
    const s = useVoiceStore.getState();
    s.syncRoom("r1");
    s.setPeerMuted("p1", true);
    s.applySpeaking({ peerId: "p1", speaking: true, level: 0.4 });
    expect(useVoiceStore.getState().mutedPeers).toEqual({ p1: true });
    expect(useVoiceStore.getState().speaking.p1?.speaking).toBe(true);

    useVoiceStore.getState().setPeerMuted("p1", false);
    expect(useVoiceStore.getState().mutedPeers).toEqual({});

    useVoiceStore.getState().setPeerMuted("p2", true);
    useVoiceStore.getState().syncRoom("r1");
    expect(useVoiceStore.getState().mutedPeers).toEqual({ p2: true });
    useVoiceStore.getState().syncRoom(null);
    expect(useVoiceStore.getState()).toMatchObject({ roomId: null, mutedPeers: {}, speaking: {} });
  });

  it("self events feed the level meter; identical events keep the state", () => {
    useVoiceStore.getState().applySpeaking({ peerId: SELF_PEER_ID, speaking: true, level: 0.7 });
    expect(useVoiceStore.getState().selfLevel).toBe(0.7);
    const before = useVoiceStore.getState();
    useVoiceStore.getState().applySpeaking({ peerId: SELF_PEER_ID, speaking: true, level: 0.7 });
    expect(useVoiceStore.getState()).toBe(before);
  });

  it("voice://status: stopping voice / mic test clears stale indicators", () => {
    const s = useVoiceStore.getState();
    s.applySpeaking({ peerId: "p1", speaking: true, level: 1 });
    s.applySpeaking({ peerId: SELF_PEER_ID, speaking: true, level: 0.5 });
    const before = useVoiceStore.getState();
    useVoiceStore.getState().applyStatus(status());
    expect(useVoiceStore.getState()).toBe(before);

    useVoiceStore.getState().applyStatus(status({ active: false, micTest: true }));
    expect(useVoiceStore.getState()).toMatchObject({ speaking: {}, selfLevel: 0.5 });
    useVoiceStore.getState().applyStatus(status({ active: false, micTest: false }));
    expect(useVoiceStore.getState().selfLevel).toBe(0);
  });
});
