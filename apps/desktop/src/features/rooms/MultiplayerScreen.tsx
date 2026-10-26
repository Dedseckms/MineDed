import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { Peer, RoomState } from "../../bindings";
import { useKickPeer, useLeaveRoom, useRoomState, useVoiceStatus } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Card, CardList, Section } from "../../components/ui/Card";
import { CopyButton } from "../../components/ui/CopyButton";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { PATH_TONE } from "../../lib/rooms";
import { isSpeaking } from "../../lib/voice";
import { useNavStore } from "../../stores/navStore";
import { useVoiceStore } from "../../stores/voiceStore";
import { PeerVoiceControls, SpeakingAvatar, VoiceControls } from "../voice/VoicePanel";
import { RoomChat } from "./RoomChat";
import { CreateRoomForm, CreateVoiceRoomForm, JoinRoomForm } from "./RoomForms";

function RoomHeader({ room, onLeave }: { room: RoomState; onLeave: () => void }) {
  const { t, i18n } = useTranslation();
  const go = useNavStore((s) => s.go);
  const host = room.role === "host";
  const voice = room.kind === "voice";
  const title = voice
    ? host ? t("rooms.room.voiceHostTitle") : t("rooms.room.voiceGuestTitle")
    : host ? t("rooms.room.hostTitle") : t("rooms.room.guestTitle");
  return (
    <Section
      title={title}
      actions={
        <Button size="sm" variant="danger" icon={<Icon name="logout" />} onClick={onLeave}>
          {host ? t("rooms.room.close") : t("rooms.room.leave")}
        </Button>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={host ? "accent" : "info"}>{t(`rooms.role.${room.role}`)}</Badge>
        <Badge tone={voice ? "info" : "neutral"}>{t(`rooms.kind.${room.kind}`)}</Badge>
        {voice ? null : <Badge>{t(`rooms.authMode.${room.authMode}`)}</Badge>}
        {room.passwordProtected ? (
          <Badge tone="warning">
            <span className="inline-flex items-center gap-1">
              <Icon name="lock" className="size-3" />
              {t("rooms.room.passwordProtected")}
            </span>
          </Badge>
        ) : null}
        {room.sig ? (<Badge>
          {room.sig.mc} · {t(`loader.${room.sig.loader}`)}
          {room.sig.loaderVersion ? ` ${room.sig.loaderVersion}` : ""}
        </Badge>) : null}
        <span className="text-xs text-muted">
          {t("rooms.room.expires", { date: new Date(room.expiresAt).toLocaleString(i18n.language) })}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-3 rounded-lg bg-surface-2 p-3">
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="text-xs text-muted">{t("rooms.room.code")}</span>
          <span className="selectable font-mono text-2xl font-semibold tracking-widest" data-testid="room-code">
            {room.code}
          </span>
        </div>
        <CopyButton value={room.code} label={t("rooms.room.copyCode")} />
        <CopyButton value={room.link} label={t("common.copyLink")} />
      </div>

      {host && !voice && room.worldOpen != null ? (
        room.worldOpen ? (
          <div role="status" data-testid="world-open" className="flex items-center gap-2 rounded-lg border border-accent/30 bg-accent/10 px-3 py-2 text-sm">
            <Icon name="check" className="size-4 shrink-0 text-accent" />
            {t("rooms.room.worldOpen")}
          </div>
        ) : (
          <div role="status" data-testid="world-closed" className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm">
            <Icon name="alert" className="size-4 shrink-0 text-warning" />
            {t("rooms.room.openWorld")}
          </div>
        )
      ) : null}

      {room.localAddr ? (
        <div className="flex flex-col gap-2 rounded-lg border border-accent/30 bg-accent/10 p-3">
          <p className="text-sm" data-testid="server-added">
            {room.peers.length === 1 && room.peers[0]
              ? t("rooms.room.serverAdded", { host: room.peers[0].name })
              : t("rooms.room.serverAddedGeneric")}
          </p>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex min-w-0 flex-1 flex-col">
              <span className="text-xs text-muted">{t("rooms.room.localAddr")}</span>
              <span className="selectable font-mono text-lg font-semibold" data-testid="room-local-addr">
                {room.localAddr}
              </span>
            </div>
            <CopyButton value={room.localAddr} />
          </div>
          <p className="text-xs text-muted">{t("rooms.room.localAddrHint")}</p>
        </div>
      ) : null}

      {voice ? (
        <p className="text-xs text-muted">{host ? t("rooms.room.voiceHostHint") : t("rooms.room.voiceGuestHint")}</p>
      ) : (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-xs text-muted">{host ? t("rooms.room.hostHint") : t("rooms.room.guestHint")}</p>
          <Button size="sm" variant="primary" icon={<Icon name="play" />} onClick={() => go("play")}>
            {t("rooms.room.toPlay")}
          </Button>
        </div>
      )}
    </Section>
  );
}

function PeerRow({ peer, canKick, onKick }: { peer: Peer; canKick: boolean; onKick: () => void }) {
  const { t } = useTranslation();
  const voiceOn = useVoiceStatus().data?.active ?? false;
  const speaking = useVoiceStore((s) => voiceOn && isSpeaking(peer, s.speaking));
  return (
    <Card className="flex flex-col gap-2 py-3">
      <div className="flex items-center gap-3">
        <SpeakingAvatar name={peer.name} speaking={speaking} />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 truncate font-medium">
            {peer.name}
            {peer.muted ? (
              <span title={t("voice.peer.micOff")} aria-label={t("voice.peer.micOff")} className="text-muted">
                <Icon name="micOff" className="size-3.5" />
              </span>
            ) : null}
          </p>
          <p className="truncate text-xs text-muted">
            {peer.rttMs !== null ? t("rooms.peers.rtt", { ms: peer.rttMs }) : t("rooms.peers.rttUnknown")}
          </p>
        </div>
        <Badge tone={PATH_TONE[peer.path]}>
          <span title={t(`rooms.pathHint.${peer.path}`)}>{t(`rooms.path.${peer.path}`)}</span>
        </Badge>
        {canKick ? (
          <Button size="sm" variant="ghost" className="text-danger" onClick={onKick} aria-label={t("rooms.peers.kickNamed", { name: peer.name })}>
            {t("rooms.peers.kick")}
          </Button>
        ) : null}
      </div>
      {voiceOn && peer.path !== "connecting" ? <PeerVoiceControls peer={peer} /> : null}
    </Card>
  );
}

function PeersSection({ room }: { room: RoomState }) {
  const { t } = useTranslation();
  const kick = useKickPeer();
  const [kicking, setKicking] = useState<Peer | null>(null);
  const host = room.role === "host";
  return (
    <Section title={t("rooms.peers.title", { n: room.peers.length })}>
      {room.peers.length === 0 ? (
        <EmptyState icon="users" title={host ? t("rooms.peers.waiting") : t("rooms.peers.none")} />
      ) : (
        <CardList className="flex flex-col gap-2">
          {room.peers.map((p) => (
            <PeerRow
              key={p.peerId}
              peer={p}
              canKick={host}
              onKick={() => {
                kick.reset();
                setKicking(p);
              }}
            />
          ))}
        </CardList>
      )}
      <ConfirmDialog
        open={kicking !== null}
        danger
        title={t("rooms.peers.kickTitle")}
        message={t("rooms.peers.kickMessage", { name: kicking?.name ?? "" })}
        confirmLabel={t("rooms.peers.kick")}
        loading={kick.isPending}
        error={kick.error ? <ErrorMessage error={kick.error} /> : null}
        onCancel={() => setKicking(null)}
        onConfirm={() => {
          if (kicking) kick.mutate(kicking.peerId, { onSuccess: () => setKicking(null) });
        }}
      />
    </Section>
  );
}

function RoomView({ room }: { room: RoomState }) {
  const { t } = useTranslation();
  const leave = useLeaveRoom();
  const [confirm, setConfirm] = useState(false);
  const host = room.role === "host";
  return (
    <div className="flex flex-col gap-6">
      <RoomHeader
        room={room}
        onLeave={() => {
          leave.reset();
          setConfirm(true);
        }}
      />
      <div className="grid gap-6 lg:grid-cols-2">
        <div className="flex flex-col gap-6">
          <VoiceControls />
          <PeersSection room={room} />
        </div>
        <RoomChat />
      </div>
      <ConfirmDialog
        open={confirm}
        danger
        title={host ? t("rooms.room.closeTitle") : t("rooms.room.leaveTitle")}
        message={host ? t("rooms.room.closeMessage") : room.kind === "voice" ? t("rooms.room.voiceLeaveMessage") : t("rooms.room.leaveMessage")}
        confirmLabel={host ? t("rooms.room.close") : t("rooms.room.leave")}
        loading={leave.isPending}
        error={leave.error ? <ErrorMessage error={leave.error} /> : null}
        onCancel={() => setConfirm(false)}
        onConfirm={() => leave.mutate(undefined, { onSuccess: () => setConfirm(false) })}
      />
    </div>
  );
}

export function MultiplayerScreen() {
  const room = useRoomState();
  if (room.isPending) return <ListSkeleton rows={2} rowClassName="h-56" className="grid gap-6 lg:grid-cols-2" />;
  if (room.error) return <ErrorMessage error={room.error} onRetry={() => void room.refetch()} />;
  if (room.data) return <RoomView room={room.data} />;
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <CreateRoomForm />
      <div className="flex flex-col gap-6">
        <JoinRoomForm />
        <CreateVoiceRoomForm />
      </div>
    </div>
  );
}
