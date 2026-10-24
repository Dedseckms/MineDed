import { useEffect } from "react";

import { useVoiceStatus } from "../api/queries";
import { type Cue, playCue, pttCue } from "../lib/sound";
import { SELF_PEER_ID } from "../lib/voice";
import { usePrefsStore } from "../stores/prefsStore";
import { useVoiceStore } from "../stores/voiceStore";

/**
 * Plays a soft cue when push-to-talk starts/stops transmitting, so players get
 * feedback in exclusive fullscreen where the overlay is not visible.
 * Mounted once in the main window (the overlay has its own JS context).
 */
export function usePttSoundCue(play: (cue: Cue) => void = playCue): void {
  const enabled = usePrefsStore((s) => s.pttSound);
  const status = useVoiceStatus().data;
  const armed =
    enabled && !!status?.active && status.mode === "pushToTalk" && !status.selfMuted && !status.micTest;

  useEffect(() => {
    if (!armed) return;
    let prev = useVoiceStore.getState().speaking[SELF_PEER_ID]?.speaking ?? false;
    return useVoiceStore.subscribe((s) => {
      const next = s.speaking[SELF_PEER_ID]?.speaking ?? false;
      const cue = pttCue(prev, next);
      prev = next;
      if (cue) play(cue);
    });
  }, [armed, play]);
}
