import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { unwrap } from "../../api/client";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { ErrorMessage } from "../../components/ui/feedback";
import { Field, TextInput } from "../../components/ui/form";
import {
  ACCOUNT_NOTE_MAX,
  ACCOUNT_PASSWORD_MAX,
  normalizeNote,
  validateAccountLogin,
  validateAccountPassword,
} from "../../lib/accountRequest";
import { rememberRequestedLogin } from "../../lib/guestSession";
import { socialRequestAccountWithCredentials } from "../../lib/socialAdmin";
import { useToastStore } from "../../stores/toastStore";

/**
 * "Ask for an account": a guest proposes a login/password (and an optional
 * note) for the server account an admin will create. The password never
 * leaves the form except for the request itself.
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
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [reveal, setReveal] = useState(false);
  const [note, setNote] = useState("");
  const [touched, setTouched] = useState(false);
  const loginError = validateAccountLogin(login);
  const passwordError = validateAccountPassword(password);
  const send = useMutation({
    mutationFn: async () => {
      unwrap(await socialRequestAccountWithCredentials(login, password, normalizeNote(note)));
      // The guest signs in with this login once the admin creates the account.
      rememberRequestedLogin(login);
    },
    onSuccess: () => {
      useToastStore.getState().push({
        tone: "info",
        messageKey: "social.accountRequest.sentToast",
        dedupeKey: "accountRequest",
      });
      setLogin("");
      setPassword("");
      setNote("");
      setTouched(false);
      setReveal(false);
      onSent();
      onClose();
    },
  });
  const close = () => {
    send.reset();
    setPassword("");
    setReveal(false);
    onClose();
  };
  const submit = () => {
    if (send.isPending || requested) return;
    setTouched(true);
    if (loginError || passwordError) return;
    send.mutate();
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
          label={t("social.accountRequest.login")}
          hint={t("social.accountRequest.loginHint")}
          error={touched ? loginError : null}
        >
          {(id, describedBy, invalid) => (
            <TextInput
              id={id}
              aria-describedby={describedBy}
              value={login}
              maxLength={16}
              autoComplete="username"
              placeholder={t("social.accountRequest.loginPlaceholder")}
              invalid={invalid}
              onChange={(e) => setLogin(e.target.value.trim())}
            />
          )}
        </Field>
        <Field
          label={t("social.accountRequest.password")}
          error={touched ? passwordError : null}
        >
          {(id, describedBy, invalid) => (
            <div className="flex gap-2">
              <TextInput
                id={id}
                aria-describedby={describedBy}
                value={password}
                maxLength={ACCOUNT_PASSWORD_MAX}
                type={reveal ? "text" : "password"}
                autoComplete="new-password"
                invalid={invalid}
                onChange={(e) => setPassword(e.target.value)}
              />
              <Button
                variant="ghost"
                aria-label={t(
                  reveal
                    ? "social.accountRequest.hidePassword"
                    : "social.accountRequest.showPassword",
                )}
                title={t(
                  reveal
                    ? "social.accountRequest.hidePassword"
                    : "social.accountRequest.showPassword",
                )}
                icon={<Icon name={reveal ? "eyeOff" : "eye"} />}
                onClick={() => setReveal((v) => !v)}
              />
            </div>
          )}
        </Field>
        <Field
          label={t("social.accountRequest.note")}
          hint={t("social.accountRequest.noteHint", { max: ACCOUNT_NOTE_MAX })}
        >
          {(id, describedBy) => (
            <textarea
              id={id}
              aria-describedby={describedBy}
              value={note}
              rows={3}
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
