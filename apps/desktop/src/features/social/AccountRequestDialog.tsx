import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { ErrorMessage } from "../../components/ui/feedback";
import { Field } from "../../components/ui/form";
import { ACCOUNT_NOTE_MAX, normalizeNote } from "../../lib/accountRequest";
import { useToastStore } from "../../stores/toastStore";

/**
 * "Ask for an account": a guest leaves an optional note for the admin. The
 * guest token is kept, so after an invite is issued the same player can sign up.
 */
export function AccountRequestDialog({
  open,
  onClose,
  requested,
  onSent,
}: {
  open: boolean;
  onClose: () => void;
  /** A request is already on file: the submit button stays disabled. */
  requested: boolean;
  onSent: () => void;
}) {
  const { t } = useTranslation();
  const [note, setNote] = useState("");
  const send = useMutation({
    mutationFn: async (text: string) =>
      unwrap(await api.socialRequestAccount(text)),
    onSuccess: () => {
      useToastStore.getState().push({
        tone: "info",
        messageKey: "social.accountRequest.sentToast",
        dedupeKey: "accountRequest",
      });
      setNote("");
      onSent();
      onClose();
    },
  });
  const close = () => {
    send.reset();
    onClose();
  };
  const submit = () => {
    if (send.isPending || requested) return;
    send.mutate(normalizeNote(note));
  };
  return (
    <Modal
      open={open}
      onClose={close}
      title={t("social.accountRequest.title")}
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            loading={send.isPending}
            disabled={requested}
            icon={<Icon name={requested ? "check" : "shield"} />}
            onClick={submit}
          >
            {t(requested ? "social.accountRequest.sent" : "social.accountRequest.submit")}
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
        <Field
          label={t("social.accountRequest.note")}
          hint={t("social.accountRequest.noteHint", { max: ACCOUNT_NOTE_MAX })}
        >
          {(id, describedBy) => (
            <textarea
              id={id}
              aria-describedby={describedBy}
              value={note}
              rows={4}
              maxLength={ACCOUNT_NOTE_MAX}
              placeholder={t("social.accountRequest.notePlaceholder")}
              className="w-full resize-none rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text outline-none transition-colors placeholder:text-muted focus:border-accent"
              onChange={(e) => setNote(e.target.value)}
            />
          )}
        </Field>
        <p className="text-xs text-muted">
          {t("social.accountRequest.tokenHint")}
        </p>
        <ErrorMessage error={send.error} />
      </form>
    </Modal>
  );
}
