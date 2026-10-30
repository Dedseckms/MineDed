import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type {
  SocialAttachment,
  SocialContent,
  SocialMessage,
  SocialUser,
} from "../../bindings";
import { Button } from "../../components/ui/Button";
import { useResolvedTheme } from "../../hooks/useAppearance";
import { userColor } from "../../lib/social";
import { useNavStore } from "../../stores/navStore";
import { AttachmentPreview } from "./AttachmentPreview";
import { ChatAvatar } from "./ChatAvatar";
import { ReceivePack } from "./PackShare";
import { clockLabel, dayLabel, type MessageGroup } from "./chat";

export interface ChatMessageActions {
  selfUuid: string;
  canModerate: boolean;
  /** Author names above other players' groups (public chat only). */
  showAuthors: boolean;
  onOpenProfile: (user: SocialUser) => void;
  onDelete: (message: SocialMessage) => void;
  onReport: (message: SocialMessage) => void;
  onModerateDelete: (message: SocialMessage) => void;
  onSave: (attachment: SocialAttachment) => void;
}

/** Sticky "Today" / "Yesterday" / date chip between message groups. */
export function DaySeparator({ at }: { at: number }) {
  const { t, i18n } = useTranslation();
  const label = dayLabel(at, new Date(), i18n.language);
  return (
    <div className="sticky top-2 z-10 flex justify-center py-1">
      <p className="rounded-full bg-surface px-3 py-0.5 text-xs text-muted shadow-sm ring-1 ring-border">
        {"date" in label ? label.date : t(label.key)}
      </p>
    </div>
  );
}

/** One author's run of messages: a single avatar, tighter spacing, one tail. */
export function MessageGroupView({
  group,
  actions,
}: {
  group: MessageGroup;
  actions: ChatMessageActions;
}) {
  const theme = useResolvedTheme();
  return (
    <div
      className={`flex items-end gap-2 px-3 py-0.5 ${group.own ? "flex-row-reverse" : ""}`}
    >
      {group.own ? null : <ChatAvatar user={group.author} size="sm" />}
      <div
        className={`flex min-w-0 max-w-[min(100%,32rem)] flex-col ${group.own ? "items-end" : "items-start"}`}
      >
        {actions.showAuthors && !group.own ? (
          <button
            type="button"
            className="cursor-pointer px-1 pb-0.5 text-xs font-semibold hover:underline"
            style={{ color: userColor(group.author.uuid, theme) }}
            onClick={() => actions.onOpenProfile(group.author)}
          >
            {group.author.name}
          </button>
        ) : null}
        {group.messages.map((message, index) => (
          <MessageBubble
            key={message.id}
            message={message}
            own={group.own}
            last={index === group.messages.length - 1}
            actions={actions}
          />
        ))}
      </div>
    </div>
  );
}

function MessageBubble({
  message,
  own,
  last,
  actions,
}: {
  message: SocialMessage;
  own: boolean;
  last: boolean;
  actions: ChatMessageActions;
}) {
  const { t, i18n } = useTranslation();
  return (
    <article
      data-own={own}
      className={`group/bubble relative mt-0.5 max-w-full min-w-0 rounded-2xl px-2.5 py-1.5 text-sm shadow-sm ${
        own ? "bg-accent text-accent-fg" : "bg-surface-2 text-text"
      } ${last ? (own ? "rounded-br-sm" : "rounded-bl-sm") : ""}`}
    >
      {last ? (
        <span
          aria-hidden
          className={`absolute bottom-0 size-2 ${own ? "-right-1 bg-accent" : "-left-1 bg-surface-2"}`}
          style={{
            clipPath: own
              ? "polygon(0 0, 100% 100%, 0 100%)"
              : "polygon(100% 0, 100% 100%, 0 100%)",
          }}
        />
      ) : null}
      {message.deleted ? (
        <p className="py-0.5 italic opacity-70">{t("social.deleted")}</p>
      ) : (
        <MessageContent message={message} save={actions.onSave} />
      )}
      <div className="mt-0.5 flex items-center justify-end gap-1 text-[11px] opacity-70">
        {message.editedAt ? <span>{t("social.edited")}</span> : null}
        <time dateTime={message.createdAt}>
          {clockLabel(message.createdAt, i18n.language)}
        </time>
      </div>
      {message.deleted ? null : (
        <div
          role="group"
          aria-label={t("social.messageActions")}
          className="mt-0.5 flex flex-wrap justify-end gap-2 opacity-0 transition-opacity focus-within:opacity-100 group-hover/bubble:opacity-100"
        >
          {message.author.uuid === actions.selfUuid ? (
            <BubbleAction onClick={() => actions.onDelete(message)}>
              {t("common.delete")}
            </BubbleAction>
          ) : (
            <BubbleAction onClick={() => actions.onReport(message)}>
              {t("social.report")}
            </BubbleAction>
          )}
          {actions.canModerate && message.channel === "global" ? (
            <BubbleAction onClick={() => actions.onModerateDelete(message)}>
              {t("social.moderateDelete")}
            </BubbleAction>
          ) : null}
        </div>
      )}
    </article>
  );
}

function BubbleAction({
  onClick,
  children,
}: {
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="cursor-pointer rounded px-1 text-[11px] underline-offset-2 hover:underline"
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** Attachment, room invite and pack cards live inside the bubble. */
function MessageContent({
  message,
  save,
}: {
  message: SocialMessage;
  save: (attachment: SocialAttachment) => void;
}) {
  const { t } = useTranslation();
  const content: SocialContent = message.content;
  if (content.type === "text")
    return <p className="whitespace-pre-wrap break-words">{content.text}</p>;
  if (content.type === "attachment")
    return (
      <div className="min-w-0">
        {content.caption ? (
          <p className="whitespace-pre-wrap break-words">{content.caption}</p>
        ) : null}
        <div className="mt-1 min-w-0 rounded-lg bg-surface p-2 text-text ring-1 ring-border">
          <p className="truncate text-xs font-medium">
            {content.attachment.fileName}
          </p>
          <p className="text-[11px] text-muted">{content.attachment.mime}</p>
          <Button
            size="sm"
            variant="secondary"
            className="mt-1"
            onClick={() => save(content.attachment)}
          >
            {t("social.saveFile", {
              size: (content.attachment.size / 1024).toFixed(0),
            })}
          </Button>
          {content.attachment.mime.startsWith("image/") ? (
            <AttachmentPreview id={content.attachment.id} />
          ) : null}
        </div>
      </div>
    );
  if (content.type === "roomInvite")
    return (
      <div className="mt-1 rounded-lg bg-surface p-2 text-text ring-1 ring-border">
        <p>{t("social.roomInvite", { code: content.code })}</p>
        <Button
          size="sm"
          className="mt-1"
          onClick={() => useNavStore.getState().openJoin(content.code)}
        >
          {t("social.join")}
        </Button>
      </div>
    );
  return (
    <div className="mt-1 rounded-lg bg-surface p-2 text-text ring-1 ring-border">
      <p className="font-medium">{content.name}</p>
      <p className="text-xs text-muted">
        {t("social.pack", {
          mc: content.mc,
          loader: content.loader,
          mods: content.mods,
        })}
      </p>
      <ReceivePack
        pack={content}
        offline={message.author.status === "offline"}
      />
    </div>
  );
}
