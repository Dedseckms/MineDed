import { beforeEach, describe, expect, it } from "vitest";

import { SELF_PEER_ID } from "../lib/voice";
import { useVoiceStore } from "./voiceStore";

beforeEach(() => useVoiceStore.getState().reset());

describe("voice store", () => {
  it("tracks session state and resets it when the room changes", () => {
    const s = useVoiceStore.getState();
    s.syncRoom("r1");
    s.setActive(true);
    s.setSelfMuted(true);
    s.setPeerMuted("p1", true);
    s.applySpeaking({ peerId: "p1", speaking: true, level: 0.4 });
    expect(useVoiceStore.getState()).toMatchObject({ active: true, selfMuted: true, mutedPeers: { p1: true } });
    expect(useVoiceStore.getState().speaking.p1?.speaking).toBe(true);

    useVoiceStore.getState().setPeerMuted("p1", false);
    expect(useVoiceStore.getState().mutedPeers).toEqual({});

    useVoiceStore.getState().syncRoom("r1");
    expect(useVoiceStore.getState().active).toBe(true);
    useVoiceStore.getState().syncRoom(null);
    expect(useVoiceStore.getState()).toMatchObject({ roomId: null, active: false, selfMuted: false, speaking: {} });
  });

  it("self events feed the level meter; stopping clears it", () => {
    const s = useVoiceStore.getState();
    s.setMicTest(true);
    s.applySpeaking({ peerId: SELF_PEER_ID, speaking: true, level: 0.7 });
    expect(useVoiceStore.getState().selfLevel).toBe(0.7);
    const before = useVoiceStore.getState();
    useVoiceStore.getState().applySpeaking({ peerId: SELF_PEER_ID, speaking: true, level: 0.7 });
    expect(useVoiceStore.getState()).toBe(before);
    useVoiceStore.getState().setMicTest(false);
    expect(useVoiceStore.getState().selfLevel).toBe(0);
  });

  it("leaving voice clears speaking indicators", () => {
    const s = useVoiceStore.getState();
    s.setActive(true);
    s.applySpeaking({ peerId: "p1", speaking: true, level: 1 });
    useVoiceStore.getState().setActive(false);
    expect(useVoiceStore.getState().speaking).toEqual({});
  });
});
