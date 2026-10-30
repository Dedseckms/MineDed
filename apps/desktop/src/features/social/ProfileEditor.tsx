import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { SocialUser } from "../../bindings";
import { api, unwrap } from "../../api/client";
import { createTaskChannel } from "../../api/channel";
import { pickFile } from "../../api/dialog";
import { Button } from "../../components/ui/Button";
import { ErrorMessage } from "../../components/ui/feedback";
import { Field, TextInput } from "../../components/ui/form";
import { Modal } from "../../components/ui/Modal";
import { useSocialStore } from "../../stores/socialStore";
import { useTaskStore } from "../../stores/taskStore";
import { localSkinSrc } from "../skins/skinSrc";
import { TaskProgress } from "../tasks/TaskProgress";
import { ChatAvatar } from "./ChatAvatar";
import {
  ACCENTS,
  MAX_BIO,
  MAX_DISPLAY_NAME,
  profilePatch,
  validateBio,
  validateDisplayName,
} from "../../lib/socialProfile";

export interface ProfileEditorProps {
  self: SocialUser;
  onClose: () => void;
  /** Called with the saved profile; the store is updated too. */
  onSaved?: (user: SocialUser) => void;
}

/** Own-profile editor: avatar, display name, bio and accent in one save. */
export function ProfileEditor({ self, onClose, onSaved }: ProfileEditorProps) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [displayName, setDisplayName] = useState(self.displayName ?? "");
  const [bio, setBio] = useState(self.bio ?? "");
  const [accent, setAccent] = useState<string | null>(self.accent ?? null);
  const [avatarPath, setAvatarPath] = useState<string | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const task = useTaskStore((s) => (taskId ? s.tasks[taskId] : undefined));
  const nameError = validateDisplayName(displayName);
  const bioError = validateBio(bio);
  const invalid = !!nameError || !!bioError;
  const avatarPreview = avatarPath ? localSkinSrc(avatarPath) : null;

  const apply = (user: SocialUser) => {
    useSocialStore.getState().set({ self: user });
    onSaved?.(user);
  };

  const save = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.socialProfileUpdate({
          ...profilePatch({ displayName, bio, accent }),
        }),
      ),
    onSuccess: (user) => {
      apply(user);
      void qc.invalidateQueries({ queryKey: ["social"] });
      onClose();
    },
  });

  const clearAvatar = useMutation({
    mutationFn: async () => unwrap(await api.socialAvatarClear()),
    onSuccess: (user) => {
      setAvatarPath(null);
      apply(user);
      void qc.invalidateQueries({ queryKey: ["social"] });
    },
  });

  const uploadAvatar = async () => {
    setError(null);
    try {
      const path = await pickFile([
        {
          name: t("social.profile.images"),
          extensions: ["png", "jpg", "jpeg", "webp"],
        },
      ]);
      if (!path) return;
      setAvatarPath(path);
      const channel = createTaskChannel((event) => {
        useTaskStore.getState().applyEvent(event);
        if (event.type === "finished") {
          if (event.error) setError(event.error);
          void qc.invalidateQueries({ queryKey: ["social"] });
        }
      });
      setTaskId(unwrap(await api.socialAvatarSet(path, channel)));
    } catch (next) {
      setError(next);
    }
  };

  return (
    <Modal
      open
      onClose={onClose}
      title={t("social.profile.editorTitle")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            loading={save.isPending}
            disabled={invalid || clearAvatar.isPending}
            onClick={() => save.mutate()}
          >
            {t("common.save")}
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <ErrorMessage error={error ?? save.error ?? clearAvatar.error} />
        <section className="space-y-2">
          <p className="text-sm font-medium">{t("social.profile.avatar")}</p>
          <div className="flex flex-wrap items-center gap-3">
            {avatarPreview ? (
              <img
                src={avatarPreview}
                alt={t("social.profile.avatar")}
                className="size-16 rounded-full object-cover ring-1 ring-border"
              />
            ) : (
              <ChatAvatar user={self} size="lg" />
            )}
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="secondary"
                disabled={save.isPending}
                onClick={() => void uploadAvatar()}
              >
                {t("social.profile.avatarPick")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                loading={clearAvatar.isPending}
                disabled={save.isPending}
                onClick={() => clearAvatar.mutate()}
              >
                {t("social.profile.avatarRemove")}
              </Button>
            </div>
          </div>
          <p className="text-xs text-muted">{t("social.profile.avatarHint")}</p>
          {task ? <TaskProgress task={task} compact /> : null}
        </section>

        <Field
          label={t("social.profile.displayName")}
          error={nameError}
          hint={t("social.profile.displayNameHint", { max: MAX_DISPLAY_NAME })}
        >
          {(id, describedBy, isInvalid) => (
            <TextInput
              id={id}
              aria-describedby={describedBy}
              invalid={isInvalid}
              maxLength={MAX_DISPLAY_NAME + 8}
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
            />
          )}
        </Field>

        <div className="space-y-1">
          <label htmlFor="profile-bio" className="text-sm font-medium">
            {t("social.profile.bio")}
          </label>
          <textarea
            id="profile-bio"
            aria-invalid={bioError ? true : undefined}
            aria-describedby="profile-bio-hint"
            rows={3}
            value={bio}
            onChange={(e) => setBio(e.target.value)}
            className={`w-full resize-y rounded-lg border bg-surface-2 px-3 py-2 text-sm text-text outline-none transition-colors placeholder:text-muted focus:border-accent ${
              bioError ? "border-danger" : "border-border"
            }`}
          />
          <p
            id="profile-bio-hint"
            className={`text-xs ${bioError ? "text-danger" : "text-muted"}`}
          >
            {bioError
              ? t(bioError)
              : t("social.profile.bioCount", { count: bio.length, max: MAX_BIO })}
          </p>
        </div>

        <div className="space-y-2">
          <p className="text-sm font-medium">{t("social.profile.accent")}</p>
          <div
            role="radiogroup"
            aria-label={t("social.profile.accent")}
            className="flex flex-wrap gap-2"
          >
            <button
              type="button"
              role="radio"
              aria-checked={accent === null}
              aria-label={t("social.accent.default")}
              title={t("social.accent.default")}
              onClick={() => setAccent(null)}
              className={`size-8 cursor-pointer rounded-full border-2 bg-surface-2 ${
                accent === null ? "border-accent" : "border-border"
              }`}
            />
            {ACCENTS.map((option) => (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={accent === option.id}
                aria-label={t(`social.accent.${option.id}`)}
                title={t(`social.accent.${option.id}`)}
                onClick={() => setAccent(option.id)}
                style={{ backgroundColor: option.swatch }}
                className={`size-8 cursor-pointer rounded-full border-2 ${
                  accent === option.id ? "border-accent" : "border-transparent"
                }`}
              />
            ))}
          </div>
          <p className="text-xs text-muted">{t("social.profile.accentHint")}</p>
        </div>
      </div>
    </Modal>
  );
}
