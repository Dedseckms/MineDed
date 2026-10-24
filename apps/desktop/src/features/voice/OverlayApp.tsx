import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { useRoomState, useVoiceSettings, useVoiceStatus } from "../../api/queries";
import { Icon } from "../../components/ui/Icon";
import { useApplyAppearance } from "../../hooks/useAppearance";
import { useRoomSync } from "../../hooks/useBackendSync";
import { closeOverlaySelf } from "../../lib/overlay";
import { SELF_PEER_ID, isSpeaking, keyLabel, lastLines } from "../../lib/voice";
import { useRoomStore } from "../../stores/roomStore";
import { useVoiceStore } from "../../stores/voiceStore";
import { SpeakingAvatar } from "./VoicePanel";

/**
 * Compact overlay for the always-on-top frameless window: participants with
 * speaking state and the last chat lines. Semi-transparent background, no
 * blur (cheap on every machine, and light mode forbids backdrop-filter anyway).
 */
export function OverlayApp() {
  const { t } = useTranslation();
  useApplyAppearance();
  useRoomSync(() => true);
  const room = useRoomState();
  const settings = useVoiceSettings();
  const voice = useVoiceStatus();
  const speaking = useVoiceStore((s) => s.speaking);
  const messages = useRoomStore((s) => s.messages);
  const lines = lastLines(messages);

  useEffect(() => {
    document.documentElement.dataset.overlay = "true";
    return () => {
      delete document.documentElement.dataset.overlay;
    };
  }, []);

  const peers = room.data?.peers ?? [];
  const mode = voice.data?.active ? voice.data.mode : settings.data?.mode;
  const ptt = mode === "pushToTalk" && settings.data?.pttKey ? keyLabel(settings.data.pttKey) : null;

  return (
    <div className="flex h-full flex-col gap-2 overflow-hidden rounded-xl border border-border/60 bg-bg/75 p-2 text-sm text-text">
      <header data-tauri-drag-region className="flex cursor-move items-center gap-2 px-1">
        <Icon name="headphones" className="pointer-events-none size-4 text-muted" />
        <span data-tauri-drag-region className="flex-1 truncate text-xs font-semibold">
          {room.data ? room.data.code : t("voice.overlay.noRoom")}
        </span>
        {ptt ? <span className="pointer-events-none text-xs text-muted">{t("voice.overlay.ptt", { key: ptt })}</span> : null}
        <button
          type="button"
          onClick={() => void closeOverlaySelf()}
          aria-label={t("voice.overlay.close")}
          className="grid size-6 cursor-pointer place-items-center rounded-md text-muted hover:bg-surface-2 hover:text-text"
        >
          <Icon name="x" className="size-3.5" />
        </button>
      </header>

      <ul aria-label={t("voice.overlay.participants")} className="flex flex-col gap-1">
        <li className="flex items-center gap-2">
          <SpeakingAvatar size="sm" name={t("rooms.chat.you")} speaking={speaking[SELF_PEER_ID]?.speaking ?? false} />
          <span className="flex-1 truncate text-muted">{t("rooms.chat.you")}</span>
          {voice.data?.selfMuted ? <Icon name="micOff" className="size-3.5 text-danger" /> : null}
        </li>
        {peers.map((p) => (
          <li key={p.peerId} className="flex items-center gap-2">
            <SpeakingAvatar size="sm" name={p.name} speaking={isSpeaking(p, speaking)} />
            <span className="flex-1 truncate">{p.name}</span>
            {p.muted ? <Icon name="micOff" className="size-3.5 text-muted" /> : null}
          </li>
        ))}
      </ul>

      {lines.length > 0 ? (
        <ol aria-label={t("rooms.chat.title")} className="mt-auto flex flex-col gap-0.5 border-t border-border/60 pt-1.5 text-xs">
          {lines.map((m) => (
            <li key={m.id} className="truncate">
              <span className="font-semibold">{m.fromName}: </span>
              {m.text}
            </li>
          ))}
        </ol>
      ) : (
        <p className="mt-auto border-t border-border/60 pt-1.5 text-[0.7rem] leading-snug text-muted">{t("voice.overlay.fullscreenHint")}</p>
      )}
    </div>
  );
}
