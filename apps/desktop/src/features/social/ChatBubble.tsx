import { useTranslation } from "react-i18next";

import type {
  SocialAttachment,
  SocialContent,
  SocialMessage,
  SocialUser,
} from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Icon, type IconName } from "../../components/ui/Icon";
import { useResolvedTheme } from "../../hooks/useAppearance";
import { userColor } from "../../lib/social";
import { accentColor, displayNameOf } from "../../lib/socialProfile";
import { useNavStore } from "../../stores/navStore";
import { AttachmentGrid, isImage } from "./AttachmentPreview";
import { ChatAvatar } from "./ChatAvatar";
import { ReceivePack } from "./PackShare";
import { clockLabel, dayLabel, type MessageGroup, segmentsOf } from "./chat";

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
  const segments = segmentsOf(group.messages);
  return (
    <div
      className={`flex items-end gap-2 px-3 py-0.5 ${group.own ? "flex-row-reverse" : ""}`}
    >
      {group.own ? null : <ChatAvatar user={group.author} size="sm" />}
      <div
        className={`flex min-w-0 max-w-[70%] flex-col ${group.own ? "items-end" : "items-start"}`}
      >
        {actions.showAuthors && !group.own ? (
          <button
            type="button"
            className="cursor-pointer px-1 pb-0.5 text-xs font-semibold hover:underline"
            style={{
              color:
                accentColor(group.author.accent, theme) ??
                userColor(group.author.uuid, theme),
            }}
            onClick={() => actions.onOpenProfile(group.author)}
          >
            {displayNameOf(group.author)}
          </button>
        ) : null}
        {segments.map((segment, index) =>
          segment.kind === "images" ? (
            <ImageBubble
              key={segment.key}
              messages={segment.messages}
              own={group.own}
              last={index === segments.length - 1}
              actions={actions}
            />
          ) : (
            <MessageBubble
              key={segment.key}
              message={segment.message}
              own={group.own}
              last={index === segments.length - 1}
              actions={actions}
            />
          ),
        )}
      </div>
    </div>
  );
}

/** Bubble shell: shrink-wraps its content, accent for own messages, one tail. */
function bubbleClass(own: boolean, last: boolean, padding: string): string {
  return `group/bubble relative mt-0.5 w-fit max-w-full min-w-0 rounded-2xl ${padding} text-sm leading-snug shadow-sm hover:z-20 ${
    own ? "bg-accent text-accent-fg" : "bg-surface-2 text-text"
  } ${last ? (own ? "rounded-br-sm" : "rounded-bl-sm") : ""} ${
    // Touch has no hover: a tap (or the OS long-press menu) reveals the actions pill.
    "focus-within:z-20 focus-within:[&_[data-testid=message-actions]]:pointer-events-auto focus-within:[&_[data-testid=message-actions]]:flex"
  }`;
}

function Tail({ own, background }: { own: boolean; background: string }) {
  return (
    <span
      aria-hidden
      className={`absolute bottom-0 size-2 ${own ? "-right-1" : "-left-1"} ${background}`}
      style={{
        clipPath: own
          ? "polygon(0 0, 100% 100%, 0 100%)"
          : "polygon(100% 0, 100% 100%, 0 100%)",
      }}
    />
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
  const { t } = useTranslation();
  return (
    <article
      data-own={own}
      data-kind="text"
      className={bubbleClass(own, last, "px-2.5 py-1")}
    >
      {last ? <Tail own={own} background={own ? "bg-accent" : "bg-surface-2"} /> : null}
      <div className="flex items-end gap-2">
        <div className="min-w-0">
          {message.deleted ? (
            <p className="italic opacity-70">{t("social.deleted")}</p>
          ) : (
            <MessageContent message={message} save={actions.onSave} />
          )}
        </div>
        <BubbleMeta message={message} />
      </div>
      <MessageActions message={message} own={own} actions={actions} />
    </article>
  );
}

/** Run of captionless images: one framed preview grid instead of N bubbles. */
function ImageBubble({
  messages,
  own,
  last,
  actions,
}: {
  messages: SocialMessage[];
  own: boolean;
  last: boolean;
  actions: ChatMessageActions;
}) {
  const attachments = messages.flatMap((message) =>
    message.content.type === "attachment" ? [message.content.attachment] : [],
  );
  const latest = messages[messages.length - 1];
  if (!latest || attachments.length === 0) return null;
  return (
    <article
      data-own={own}
      data-kind="images"
      className={bubbleClass(own, last, "p-1")}
    >
      {last ? <Tail own={own} background={own ? "bg-accent" : "bg-surface-2"} /> : null}
      <AttachmentGrid attachments={attachments} onSave={actions.onSave} />
      <BubbleMeta message={latest} className="justify-end px-1 pt-0.5" />
      <MessageActions message={latest} own={own} actions={actions} />
    </article>
  );
}

/** Compact time line trailing the last line of the bubble. */
function BubbleMeta({
  message,
  className = "",
}: {
  message: SocialMessage;
  className?: string;
}) {
  const { t, i18n } = useTranslation();
  return (
    <span
      className={`flex shrink-0 items-center gap-1 text-[10px] leading-4 whitespace-nowrap opacity-60 ${className}`}
    >
      {message.editedAt ? <span>{t("social.edited")}</span> : null}
      <time dateTime={message.createdAt}>
        {clockLabel(message.createdAt, i18n.language)}
      </time>
    </span>
  );
}

type ActionKind = "delete" | "report" | "moderateDelete";

/**
 * Hover/focus pill pinned to the bubble's outer edge, outside the text flow:
 * absolutely positioned with no layout size, so it never shifts messages or
 * changes the bubble height. Touch devices get it on long press instead.
 */
function MessageActions({
  message,
  own,
  actions,
}: {
  message: SocialMessage;
  own: boolean;
  actions: ChatMessageActions;
}) {
  const { t } = useTranslation();
  if (message.deleted) return null;
  const ownMessage = message.author.uuid === actions.selfUuid;
  const primary: ActionKind = ownMessage ? "delete" : "report";
  const canModerate = actions.canModerate && message.channel === "global";
  // At most two icon buttons: a moderator's "delete as moderator" is the second.
  const pinned: ActionKind[] = canModerate ? [primary, "moderateDelete"] : [primary];
  const run = (kind: ActionKind) => {
    if (kind === "delete") actions.onDelete(message);
    else if (kind === "report") actions.onReport(message);
    else actions.onModerateDelete(message);
  };
  const label: Record<ActionKind, string> = {
    delete: t("social.deleteMessage"),
    report: t("social.reportMessage"),
    moderateDelete: t("social.moderateDelete"),
  };
  const icon: Record<ActionKind, IconName> = {
    delete: "trash",
    report: "flag",
    moderateDelete: "shield",
  };
  return (
    <div
      role="group"
      aria-label={t("social.messageActions")}
      data-testid="message-actions"
      // Absolutely placed and transient: zero layout size, so no bubble moves.
      // The padding bridges the gap to the bubble, otherwise moving the pointer
      // into the pill leaves the bubble and hides it.
      className={`pointer-events-none absolute top-0 z-10 hidden items-center rounded-full px-1.5 py-1 group-focus-within/bubble:pointer-events-auto group-focus-within/bubble:flex group-hover/bubble:pointer-events-auto group-hover/bubble:flex ${
        // Right-aligned bubbles grow leftwards, so their outer edge is on the left.
        own ? "right-full" : "left-full"
      }`}
    >
      <div className="pointer-events-auto flex items-center gap-0.5 rounded-full bg-surface/95 p-0.5 shadow-sm ring-1 ring-border">
        {pinned.map((kind) => (
          <BubbleAction key={kind} icon={icon[kind]} label={label[kind]} onClick={() => run(kind)} />
        ))}
      </div>
    </div>
  );
}

/** Small round icon button: 24 px, labelled for screen readers and tooltips. */
function BubbleAction({
  icon,
  label,
  onClick,
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="grid size-6 cursor-pointer place-items-center rounded-full text-muted transition-colors hover:bg-surface-2 hover:text-text focus-visible:bg-surface-2"
    >
      <Icon name={icon} className="size-3.5" />
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
  if (content.type === "attachment") {
    // Images render as previews only; saving lives in the full-screen viewer.
    return (
      <div className="min-w-0">
        {isImage(content.attachment) ? (
          <AttachmentGrid attachments={[content.attachment]} onSave={save} />
        ) : (
          <div className="min-w-0 rounded-lg bg-surface p-2 text-text ring-1 ring-border">
            <p className="truncate text-xs font-medium">
              {content.attachment.fileName}
            </p>
            <p className="text-[11px] text-muted">
              {(content.attachment.size / 1024).toFixed(0)} KiB
            </p>
            <Button
              size="sm"
              variant="secondary"
              className="mt-1"
              onClick={() => save(content.attachment)}
            >
              {t("common.save")}
            </Button>
          </div>
        )}
        {content.caption ? (
          <p className="mt-1 whitespace-pre-wrap break-words">{content.caption}</p>
        ) : null}
      </div>
    );
  }
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
