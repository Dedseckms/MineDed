import { useTranslation } from "react-i18next";

import type { SocialUser } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Badge } from "../../components/ui/feedback";
import { useMotionPrefs, useResolvedTheme } from "../../hooks/useAppearance";
import { canContact, userColor } from "../../lib/social";
import {
  accentColor,
  accentOption,
  canEditProfile,
  displayNameOf,
} from "../../lib/socialProfile";
import { Skin3D, SkinFlat } from "../skins/SkinPreview";
import { ChatAvatar } from "./ChatAvatar";

export interface ProfileCardProps {
  user: SocialUser;
  self: SocialUser;
  blocked: boolean;
  canInvite: boolean;
  onEdit?: () => void;
  onWrite: () => void;
  onInvite: () => void;
  onToggleFriend: () => void;
  onToggleBlock: () => void;
  /** The viewer is a guest: no direct chats and no friend requests. */
  guest?: boolean;
}

/** Profile card shown from chat: avatar, display name, bio and accent. */
export function ProfileCard({
  user,
  self,
  blocked,
  canInvite,
  onEdit,
  onWrite,
  onInvite,
  onToggleFriend,
  onToggleBlock,
  guest = false,
}: ProfileCardProps) {
  const { t } = useTranslation();
  const theme = useResolvedTheme();
  const { light, reduced } = useMotionPrefs();
  const name = displayNameOf(user);
  const accent = accentOption(user.accent);
  const own = user.uuid === self.uuid;
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3">
        <ChatAvatar user={user} size="lg" />
        <div className="min-w-0 flex-1">
          <p
            className="truncate text-lg font-semibold"
            style={{
              color: accentColor(user.accent, theme) ?? userColor(user.uuid, theme),
            }}
          >
            {name}
          </p>
          {name !== user.name ? (
            <p className="truncate text-xs text-muted">@{user.name}</p>
          ) : null}
          <div className="mt-1 flex flex-wrap items-center gap-1">
            <Badge tone={user.role === "admin" ? "accent" : "neutral"}>
              {t(`social.role.${user.role}`)}
            </Badge>
            {user.guest ? (
              <Badge tone="warning">{t("social.profile.guest")}</Badge>
            ) : null}
            {accent ? (
              <span className="inline-flex items-center gap-1 text-[11px] text-muted">
                <span
                  aria-hidden
                  className="size-2.5 rounded-full"
                  style={{ backgroundColor: accent.swatch }}
                />
                {t(`social.accent.${accent.id}`)}
              </span>
            ) : null}
          </div>
        </div>
      </div>
      {user.bio ? (
        <p className="rounded-lg bg-surface-2 px-3 py-2 text-sm whitespace-pre-wrap break-words">
          {user.bio}
        </p>
      ) : null}
      {user.skinUrl ? (
        light ? (
          <SkinFlat url={user.skinUrl} model="classic" className="h-32" />
        ) : (
          <Skin3D
            url={user.skinUrl}
            model="classic"
            capeUrl={null}
            animate={!reduced}
            className="h-48"
          />
        )
      ) : null}
      <p className="text-sm text-muted">
        {t(`social.status.${user.status ?? "hidden"}`)}
      </p>
      <div className="flex flex-wrap gap-2">
        {own && canEditProfile(self) && onEdit ? (
          <Button onClick={onEdit}>{t("social.profile.edit")}</Button>
        ) : null}
        {own || guest ? null : (
          <>
            <Button disabled={!canContact(user, blocked)} onClick={onWrite}>
              {t("social.write")}
            </Button>
            <Button variant="secondary" disabled={!canInvite || blocked} onClick={onInvite}>
              {t("social.invite")}
            </Button>
            <Button variant="secondary" onClick={onToggleFriend}>
              {t(user.isFriend ? "social.removeFriend" : "social.addFriend")}
            </Button>
            <Button variant="ghost" onClick={onToggleBlock}>
              {t(blocked ? "social.unblock" : "social.block")}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
