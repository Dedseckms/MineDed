import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { AppError, SocialContent } from "../../bindings";
import { api, unwrap } from "../../api/client";
import { qk, useDownloadHostPack, useInstances } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, Select, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { applySocialEvent } from "../../hooks/useSocialSync";
import { useTaskView } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

type Pack = Extract<SocialContent, { type: "packShare" }>;

/** Native command creates the host room and publishes authoritative metadata. */
export function SharePack({ channel, disabled }: { channel: string; disabled: boolean }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [picked, setPicked] = useState("");
  const instances = useInstances();
  const id = picked || instances.data?.[0]?.id || "";
  const share = useMutation({
    mutationFn: async () => unwrap(await api.socialSharePack(channel, id)),
    onSuccess: (message) => {
      applySocialEvent(qc, { type: "message", message });
      void qc.invalidateQueries({ queryKey: qk.room });
      setOpen(false);
    },
  });
  return <>
    <Button disabled={disabled} onClick={() => { share.reset(); setOpen(true); }}>{t("social.sharePack")}</Button>
    <Modal open={open} onClose={() => setOpen(false)} title={t("social.sharePack")}>
      <div className="flex flex-col gap-3">
        <p className="text-sm text-muted">{t("social.packHostOnline")}</p>
        <Field label={t("rooms.instance")}>
          {(inputId) => <Select id={inputId} value={id} disabled={share.isPending} onChange={(e) => setPicked(e.target.value)}>
            {(instances.data ?? []).map((i) => <option key={i.id} value={i.id}>{i.name} · {i.mc}</option>)}
          </Select>}
        </Field>
        <ErrorMessage error={instances.error ?? share.error} />
        <Button variant="primary" disabled={disabled || !id} loading={share.isPending} onClick={() => share.mutate()}>{t("social.sharePackConfirm")}</Button>
      </div>
    </Modal>
  </>;
}

/** Join explicitly, then reuse the existing checked host-pack import pipeline. */
export function ReceivePack({ pack, offline }: { pack: Pack; offline: boolean }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState(false);
  const download = useDownloadHostPack((failure) => { setError(failure); setDone(!failure); });
  const task = useTaskView(download.data);
  const join = useMutation({
    mutationFn: async () => {
      setError(null);
      const room = unwrap(await api.roomState());
      if (room && (room.code !== pack.code || room.role === "host")) {
        throw { code: "conflict", message: "An unrelated room is active", details: { reason: "packRoomBusy" } } satisfies AppError;
      }
      if (!room) {
        const joined = unwrap(await api.roomJoin(pack.code ?? "", null, pack.passwordProtected ? password : null));
        qc.setQueryData(qk.room, joined);
      }
      setPassword("");
    },
    onSuccess: () => download.mutate(undefined),
    onError: () => setPassword(""),
  });
  const busy = join.isPending || download.isPending || task?.status === "running";
  const close = () => { setPassword(""); setOpen(false); };
  return <>
    <p className="text-xs text-muted">{t(offline ? "social.senderOffline" : pack.code ? "social.packHostOnline" : "social.packUnavailable")}</p>
    <Button size="sm" disabled={offline || !pack.code || done || busy} onClick={() => setOpen(true)}>{t(done ? "social.packImported" : "social.downloadPack")}</Button>
    <Modal open={open} onClose={close} title={t("social.downloadPack")}>
      <div className="flex flex-col gap-3">
        <p>{pack.name}</p>
        <p className="text-sm text-muted">{t("social.packHostOnline")}</p>
        {pack.passwordProtected ? <Field label={t("rooms.password")}>
          {(inputId) => <TextInput id={inputId} type="password" autoComplete="off" value={password} onChange={(e) => setPassword(e.target.value)} />}
        </Field> : null}
        <ErrorMessage error={join.error ?? download.error ?? error} />
        {task ? <TaskProgress task={task} compact /> : null}
        {done ? <p role="status">{t("social.packImported")}</p> : <Button variant="primary" loading={busy}
          disabled={offline || !pack.code || (pack.passwordProtected && !password)} onClick={() => join.mutate()}>{t("social.downloadPack")}</Button>}
      </div>
    </Modal>
  </>;
}
