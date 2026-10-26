import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AppError, InstanceId, InstanceSig, RoomState } from "../../bindings";
import { autoAttachHostPack, useAttachRoomInstance, useDownloadHostPack, useInstances } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { ErrorMessage } from "../../components/ui/feedback";
import { Select, Switch } from "../../components/ui/form";
import { hostPackCandidates } from "../../lib/rooms";
import { useHostPackStore } from "../../stores/hostPackStore";
import { useNavStore } from "../../stores/navStore";
import { usePrefsStore } from "../../stores/prefsStore";
import { useTaskView } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

function useSigLabel(sig: InstanceSig) {
  const { t } = useTranslation();
  return t("rooms.pack.summary", {
    loader: `${t(`loader.${sig.loader}`)}${sig.loaderVersion ? ` ${sig.loaderVersion}` : ""}`,
    mc: sig.mc,
  });
}

/** Secondary path: bind the room to an instance the user picks. */
function ManualPick({ onAttached }: { onAttached: (id: InstanceId) => void }) {
  const { t } = useTranslation();
  const instances = useInstances();
  const attachRoom = useAttachRoomInstance();
  const list = instances.data ?? [];
  const [picked, setPicked] = useState<InstanceId | null>(null);
  const value = picked && list.some((i) => i.id === picked) ? picked : (list[0]?.id ?? null);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          aria-label={t("rooms.instance")}
          className="min-w-0 flex-1"
          value={value ?? ""}
          disabled={list.length === 0}
          onChange={(e) => {
            setPicked(e.target.value);
            attachRoom.reset();
          }}
        >
          {list.map((i) => (
            <option key={i.id} value={i.id}>
              {i.name} · {i.mc}
            </option>
          ))}
        </Select>
        <Button
          size="sm"
          disabled={!value}
          loading={attachRoom.isPending}
          onClick={() => {
            if (!value) return;
            attachRoom.mutate(value, { onSuccess: () => onAttached(value) });
          }}
        >
          {t("rooms.pack.attach")}
        </Button>
      </div>
      <ErrorMessage error={attachRoom.error} />
    </div>
  );
}

/**
 * Game room guest: binds the room to a matching local instance, or offers to
 * download the host's pack (optionally automatically), then a Play button.
 */
export function HostPackSection({ room, sig }: { room: RoomState; sig: InstanceSig }) {
  const { t } = useTranslation();
  const instances = useInstances();
  const phase = useHostPackStore((s) => (s.roomId === room.roomId ? s.phase : "idle"));
  const attached = useHostPackStore((s) => (s.roomId === room.roomId ? s.instance : null));
  const autoPack = usePrefsStore((s) => s.autoHostPack);
  const setAutoPack = usePrefsStore((s) => s.setAutoHostPack);
  const selectPlayInstance = useNavStore((s) => s.selectPlayInstance);
  const go = useNavStore((s) => s.go);
  const [dismissed, setDismissed] = useState(false);
  const [manual, setManual] = useState(false);
  const [taskError, setTaskError] = useState<AppError | null>(null);
  const roomId = room.roomId;
  const sigLabel = useSigLabel(sig);

  const onAttached = (id: InstanceId) => {
    useHostPackStore.getState().attach(roomId, id);
    selectPlayInstance(id);
  };

  const download = useDownloadHostPack((error, list) => {
    const store = useHostPackStore.getState();
    if (error) {
      setTaskError(error);
      store.setPhase(roomId, "needPack");
      return;
    }
    const match = hostPackCandidates(list, sig)[0];
    if (match) onAttached(match.id);
    else store.setPhase(roomId, "needPack");
  });
  const task = useTaskView(download.data);

  const startDownload = () => {
    setTaskError(null);
    useHostPackStore.getState().setPhase(roomId, "downloading");
    download.mutate(undefined, { onError: () => useHostPackStore.getState().setPhase(roomId, "needPack") });
  };
  const startRef = useRef(startDownload);
  startRef.current = startDownload;
  const onAttachedRef = useRef(onAttached);
  onAttachedRef.current = onAttached;

  // Once per room: try a matching local instance, else offer (or start) the download.
  useEffect(() => {
    const list = instances.data;
    if (!list || !useHostPackStore.getState().begin(roomId)) return;
    void autoAttachHostPack(list, sig).then((found) => {
      if (found) onAttachedRef.current(found);
      else if (usePrefsStore.getState().autoHostPack) startRef.current();
      else useHostPackStore.getState().setPhase(roomId, "needPack");
    });
  }, [instances.data, roomId, sig]);

  const attachedInst = instances.data?.find((i) => i.id === attached) ?? null;
  const downloading = phase === "downloading";
  const needPack = phase === "needPack" || downloading;

  const downloadButton = (
    <Button variant="primary" icon={<Icon name="download" />} loading={downloading} onClick={startDownload}>
      {t("rooms.pack.download")}
    </Button>
  );
  const progress = (
    <>
      {task ? <TaskProgress task={task} compact /> : null}
      {taskError && task?.status !== "failed" ? <ErrorMessage error={taskError} /> : null}
      <ErrorMessage error={download.error} />
    </>
  );

  return (
    <Section title={t("rooms.pack.title")}>
      <p className="text-sm" data-testid="host-pack-summary">
        {sigLabel}
      </p>
      {phase === "attached" ? (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-muted" data-testid="host-pack-ready">
            {t("rooms.pack.ready", { name: attachedInst?.name ?? attached ?? "" })}
          </p>
          <Button
            variant="primary"
            icon={<Icon name="play" />}
            onClick={() => {
              if (attached) selectPlayInstance(attached);
              go("play");
            }}
          >
            {t("rooms.pack.play")}
          </Button>
        </div>
      ) : phase === "matching" || phase === "idle" ? (
        <p className="text-sm text-muted">{t("rooms.pack.matching")}</p>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted">{t("rooms.pack.noMatch")}</p>
          <div className="flex flex-wrap gap-2">{downloadButton}</div>
          {progress}
        </div>
      )}
      {phase !== "attached" ? (
        <div className="flex flex-col gap-2">
          <Button size="sm" variant="ghost" className="w-fit" aria-expanded={manual} onClick={() => setManual((v) => !v)}>
            {t("rooms.pack.manual")}
          </Button>
          {manual ? <ManualPick onAttached={onAttached} /> : null}
        </div>
      ) : null}

      <Modal
        open={needPack && !dismissed}
        onClose={() => setDismissed(true)}
        title={t("rooms.pack.title")}
        footer={
          <>
            <Button variant="ghost" onClick={() => setDismissed(true)}>
              {t("rooms.pack.later")}
            </Button>
            {downloadButton}
          </>
        }
      >
        <div className="flex flex-col gap-3">
          <p className="font-medium">{sigLabel}</p>
          <p className="text-sm text-muted">{t("rooms.pack.intro")}</p>
          <Switch checked={autoPack} onChange={setAutoPack} label={t("rooms.pack.auto")} />
          {progress}
        </div>
      </Modal>
    </Section>
  );
}
