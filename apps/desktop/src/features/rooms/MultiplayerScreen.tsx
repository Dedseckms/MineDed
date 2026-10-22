import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { Peer, RoomState } from "../../bindings";
import { useKickPeer, useLeaveRoom, useRoomState } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Card, CardList, Section } from "../../components/ui/Card";
import { CopyButton } from "../../components/ui/CopyButton";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { PATH_TONE } from "../../lib/rooms";
import { useNavStore } from "../../stores/navStore";
import { RoomChat } from "./RoomChat";
import { CreateRoomForm, JoinRoomForm } from "./RoomForms";

function RoomHeader({ room, onLeave }: { room: RoomState; onLeave: () => void }) {
  const { t, i18n } = useTranslation();
  const go = useNavStore((s) => s.go);
  const host = room.role === "host";
  return (
    <Section
      title={host ? t("rooms.room.hostTitle") : t("rooms.room.guestTitle")}
      actions={
        <Button size="sm" variant="danger" icon={<Icon name="logout" />} onClick={onLeave}>
          {host ? t("rooms.room.close") : t("rooms.room.leave")}
        </Button>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={host ? "accent" : "info"}>{t(`rooms.role.${room.role}`)}</Badge>
        <Badge>{t(`rooms.authMode.${room.authMode}`)}</Badge>
        {room.passwordProtected ? (
          <Badge tone="warning">
            <span className="inline-flex items-center gap-1">
              <Icon name="lock" className="size-3" />
              {t("rooms.room.passwordProtected")}
            </span>
          </Badge>
        ) : null}
        <Badge>
          {room.sig.mc} · {t(`loader.${room.sig.loader}`)}
          {room.sig.loaderVersion ? ` ${room.sig.loaderVersion}` : ""}
        </Badge>
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

      {room.localAddr ? (
        <div className="flex flex-col gap-2 rounded-lg border border-accent/30 bg-accent/10 p-3">
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

      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-xs text-muted">{host ? t("rooms.room.hostHint") : t("rooms.room.guestHint")}</p>
        <Button size="sm" variant="primary" icon={<Icon name="play" />} onClick={() => go("play")}>
          {t("rooms.room.toPlay")}
        </Button>
      </div>
    </Section>
  );
}

function PeerRow({ peer, canKick, onKick }: { peer: Peer; canKick: boolean; onKick: () => void }) {
  const { t } = useTranslation();
  return (
    <Card className="flex items-center gap-3 py-3">
      <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-surface-2 font-semibold uppercase">
        {peer.name.slice(0, 1)}
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">{peer.name}</p>
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
        <EmptyState title={host ? t("rooms.peers.waiting") : t("rooms.peers.none")} />
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
        <PeersSection room={room} />
        <RoomChat />
      </div>
      <ConfirmDialog
        open={confirm}
        danger
        title={host ? t("rooms.room.closeTitle") : t("rooms.room.leaveTitle")}
        message={host ? t("rooms.room.closeMessage") : t("rooms.room.leaveMessage")}
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
  if (room.isPending) return <Skeleton className="h-40" />;
  if (room.error) return <ErrorMessage error={room.error} />;
  if (room.data) return <RoomView room={room.data} />;
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <CreateRoomForm />
      <JoinRoomForm />
    </div>
  );
}
