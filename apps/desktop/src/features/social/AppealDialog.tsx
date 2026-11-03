import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { ErrorMessage } from "../../components/ui/feedback";
import { Field } from "../../components/ui/form";
import { useToastStore } from "../../stores/toastStore";

const APPEAL_MAX = 500;

/**
 * Ban appeal: a short message to the admins. A banned player has no other way
 * to reach them, so this dialog is the only channel the chat offers.
 */
export function AppealDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [touched, setTouched] = useState(false);
  const send = useMutation({
    mutationFn: async (message: string) => unwrap(await api.socialAppeal(message)),
    onSuccess: () => {
      useToastStore.getState().push({
        tone: "info",
        messageKey: "social.appeal.sent",
        dedupeKey: "appealSent",
      });
      setText("");
      setTouched(false);
      onClose();
    },
  });
  const trimmed = text.trim();
  const error = trimmed.length < 3 ? t("social.appeal.tooShort") : null;
  const submit = () => {
    setTouched(true);
    if (error) return;
    send.mutate(trimmed);
  };
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("social.appeal.title")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            loading={send.isPending}
            disabled={!!error}
            onClick={submit}
          >
            {t("social.appeal.submit")}
          </Button>
        </>
      }
    >
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <p className="text-sm text-muted">{t("social.appeal.hint")}</p>
        <Field label={t("social.appeal.text")} error={touched ? error : null}>
          {(id, describedBy, bad) => (
            <textarea
              id={id}
              aria-describedby={describedBy}
              aria-invalid={bad || undefined}
              rows={4}
              maxLength={APPEAL_MAX}
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t("social.appeal.placeholder")}
              className="w-full resize-y rounded-lg border border-border bg-surface px-3 py-2 text-sm"
            />
          )}
        </Field>
        <ErrorMessage error={send.error} />
      </form>
    </Modal>
  );
}
