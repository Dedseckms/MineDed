import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { Peer } from "../../bindings";
import {
  useSetPeerMuted,
  useSetPeerVolume,
  useSetSelfMuted,
  useSystemInfo,
  useVoiceActive,
  useVoiceSettings,
  useVoiceStatus,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { LevelMeter, PercentSlider } from "../../components/ui/LevelMeter";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { useDraftValue } from "../../hooks/useDraftValue";
import { toAppError } from "../../lib/errors";
import { toggleOverlay } from "../../lib/overlay";
import { SELF_PEER_ID, fromPercent, keyLabel, peerVolume, toPercent, voiceStatus } from "../../lib/voice";
import { useNavStore } from "../../stores/navStore";
import { useVoiceStore } from "../../stores/voiceStore";

const STATUS_TONE = {
  inactive: "neutral",
  off: "neutral",
  muted: "danger",
  pttNoKey: "warning",
  pushToTalk: "info",
  vad: "accent",
} as const;

/** Avatar with a speaking ring (CSS only; no per-frame JS). */
export function SpeakingAvatar({ name, speaking, size = "md" }: { name: string; speaking: boolean; size?: "sm" | "md" }) {
  const dims = size === "sm" ? "size-6 text-xs" : "size-9";
  return (
    <span
      data-speaking={speaking || undefined}
      className={`grid shrink-0 place-items-center rounded-lg bg-surface-2 font-semibold uppercase ring-2 transition-shadow ${dims} ${
        speaking ? "ring-accent" : "ring-transparent"
      }`}
    >
      {name.slice(0, 1)}
    </span>
  );
}

/** Mute + volume of a remote peer (visible while voice is on). */
export function PeerVoiceControls({ peer }: { peer: Peer }) {
  const { t } = useTranslation();
  const settings = useVoiceSettings();
  const muted = useVoiceStore((s) => s.mutedPeers[peer.peerId] ?? false);
  const setMuted = useSetPeerMuted();
  const setVolume = useSetPeerVolume();
  const [volume, onVolume] = useDraftValue(toPercent(peerVolume(settings.data, peer)), (p) =>
    setVolume.mutate({ peer, volume: fromPercent(p) }),
  );
  return (
    <div className="flex w-full items-center gap-2">
      <Button
        size="sm"
        variant={muted ? "danger" : "ghost"}
        icon={<Icon name={muted ? "volumeOff" : "volume"} />}
        aria-pressed={muted}
        aria-label={muted ? t("voice.peer.unmuteNamed", { name: peer.name }) : t("voice.peer.muteNamed", { name: peer.name })}
        loading={setMuted.isPending}
        onClick={() => setMuted.mutate({ peerId: peer.peerId, muted: !muted })}
      />
      <PercentSlider
        className="flex-1"
        showLabel={false}
        label={t("voice.peer.volumeNamed", { name: peer.name })}
        value={volume}
        disabled={muted}
        onChange={onVolume}
      />
      {setMuted.error || setVolume.error ? <ErrorMessage error={setMuted.error ?? setVolume.error} /> : null}
    </div>
  );
}

function OverlayButton() {
  const { t } = useTranslation();
  const [error, setError] = useState<unknown>(null);
  return (
    <div className="flex flex-col gap-1.5">
      <Button
        size="sm"
        icon={<Icon name="overlay" />}
        onClick={() => {
          setError(null);
          toggleOverlay(t("voice.overlay.title")).catch((e: unknown) => setError(toAppError(e)));
        }}
      >
        {t("voice.overlay.toggle")}
      </Button>
      <ErrorMessage error={error} />
    </div>
  );
}

/** Header shortcut while voice is on: self mute from any screen. */
export function VoiceHeaderToggle() {
  const { t } = useTranslation();
  const status = useVoiceStatus();
  const active = status.data?.active ?? false;
  const selfMuted = status.data?.selfMuted ?? false;
  const speaking = useVoiceStore((s) => s.speaking[SELF_PEER_ID]?.speaking ?? false);
  const mute = useSetSelfMuted();
  if (!active) return null;
  return (
    <Button
      size="sm"
      variant={selfMuted ? "danger" : "ghost"}
      icon={<Icon name={selfMuted ? "micOff" : "mic"} className={`size-4 ${speaking && !selfMuted ? "text-accent" : ""}`} />}
      aria-pressed={selfMuted}
      aria-label={selfMuted ? t("voice.unmuteSelf") : t("voice.muteSelf")}
      title={selfMuted ? t("voice.unmuteSelf") : t("voice.muteSelf")}
      loading={mute.isPending}
      onClick={() => mute.mutate(!selfMuted)}
    />
  );
}

/** Own voice: join/leave voice, self mute, mode status, overlay. */
export function VoiceControls() {
  const { t } = useTranslation();
  const go = useNavStore((s) => s.go);
  const settings = useVoiceSettings();
  const system = useSystemInfo();
  const voice = useVoiceStatus();
  const active = voice.data?.active ?? false;
  const selfMuted = voice.data?.selfMuted ?? false;
  const selfSpeaking = useVoiceStore((s) => s.speaking[SELF_PEER_ID]?.speaking ?? false);
  const selfLevel = useVoiceStore((s) => s.selfLevel);
  const toggle = useVoiceActive();
  const mute = useSetSelfMuted();
  // Effective mode from the backend (PTT may fall back to VAD, e.g. on Wayland).
  const mode = active ? voice.data?.mode : settings.data?.mode;
  const pttKey = settings.data?.pttKey ?? null;
  const status = voiceStatus(mode, active, selfMuted, pttKey);

  return (
    <Section
      title={t("voice.title")}
      actions={
        <Button size="sm" variant="ghost" icon={<Icon name="settings" />} onClick={() => go("settings")}>
          {t("voice.openSettings")}
        </Button>
      }
    >
      <div className="flex flex-wrap items-center gap-3">
        <SpeakingAvatar name={t("rooms.chat.you")} speaking={active && selfSpeaking} />
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <span data-testid="voice-status">
            <Badge tone={STATUS_TONE[status]}>{t(`voice.status.${status}`, { key: pttKey ? keyLabel(pttKey) : "" })}</Badge>
          </span>
          {active ? <LevelMeter level={selfMuted ? 0 : selfLevel} label={t("voice.selfLevel")} /> : null}
        </div>
        {active ? (
          <Button
            size="sm"
            variant={selfMuted ? "danger" : "secondary"}
            icon={<Icon name={selfMuted ? "micOff" : "mic"} />}
            aria-pressed={selfMuted}
            loading={mute.isPending}
            onClick={() => mute.mutate(!selfMuted)}
          >
            {selfMuted ? t("voice.unmuteSelf") : t("voice.muteSelf")}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant={active ? "danger" : "primary"}
          icon={<Icon name="headphones" />}
          loading={toggle.isPending}
          onClick={() => toggle.mutate(!active)}
        >
          {active ? t("voice.leave") : t("voice.join")}
        </Button>
      </div>
      {active && mode === "pushToTalk" && system.data?.wayland ? (
        <p role="note" className="text-xs text-warning">
          {t("voice.settings.waylandPtt")}
        </p>
      ) : null}
      <ErrorMessage error={toggle.error ?? mute.error} />
      <div className="flex flex-col gap-1.5 border-t border-border pt-3">
        <OverlayButton />
        <p className="text-xs text-muted">{t(system.data?.wayland ? "voice.overlay.hintWayland" : "voice.overlay.hint")}</p>
      </div>
    </Section>
  );
}
