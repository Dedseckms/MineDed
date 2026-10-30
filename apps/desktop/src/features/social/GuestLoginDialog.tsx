import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import type { SocialUser } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Modal } from "../../components/ui/Modal";
import { Field, TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { NICK_MAX, validateNick } from "../../lib/validation";

/**
 * "Enter as a guest": an offline nickname gets a limited hub session. Guests
 * read the public chat only, so the dialog explains the restriction.
 */
export function GuestLoginDialog({
  open,
  onClose,
  onLogin,
}: {
  open: boolean;
  onClose: () => void;
  onLogin: (user: SocialUser) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const login = useMutation({
    mutationFn: async (nick: string) =>
      unwrap(await api.socialGuestLogin(nick.trim())),
    onSuccess: (user) => {
      setName("");
      setTouched(false);
      onLogin(user);
      onClose();
    },
  });
  const invalid = validateNick(name.trim());
  /** Clears the form so the next visitor starts fresh. */
  const close = () => {
    setName("");
    setTouched(false);
    login.reset();
    onClose();
  };
  const submit = () => {
    setTouched(true);
    if (invalid) return;
    login.mutate(name);
  };
  return (
    <Modal
      open={open}
      onClose={close}
      title={t("social.guestLogin")}
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            loading={login.isPending}
            disabled={!!invalid}
            onClick={submit}
          >
            {t("social.guestSubmit")}
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
          label={t("social.guestName")}
          error={touched ? invalid : null}
          hint={t("social.guestHint")}
        >
          {(id, describedBy, bad) => (
            <TextInput
              id={id}
              aria-describedby={describedBy}
              invalid={bad}
              value={name}
              maxLength={NICK_MAX}
              autoFocus
              placeholder={t("social.guestNamePlaceholder")}
              onChange={(e) => {
                setName(e.target.value);
                setTouched(true);
                login.reset();
              }}
            />
          )}
        </Field>
        <ErrorMessage error={login.error} />
      </form>
    </Modal>
  );
}
