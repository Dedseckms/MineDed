import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { SocialChannel, SocialUser } from "../../bindings";
import { Icon } from "../../components/ui/Icon";
import { Badge } from "../../components/ui/feedback";
import { ChatAvatar } from "./ChatAvatar";
import { clockLabel, previewOf } from "./chat";

export interface ChatChannelListProps {
  channels: SocialChannel[];
  selected: string;
  onSelect: (channel: SocialChannel) => void;
  /** Player search results for the current query. */
  users: SocialUser[];
  searching: boolean;
  search: string;
  onSearch: (value: string) => void;
  onOpenProfile: (user: SocialUser) => void;
}

/** Telegram-style left column: search, sections, unread badges. */
export function ChatChannelList({
  channels,
  selected,
  onSelect,
  users,
  searching,
  search,
  onSearch,
  onOpenProfile,
}: ChatChannelListProps) {
  const { t } = useTranslation();
  const term = search.trim().toLocaleLowerCase();
  const global = channels.filter((c) => c.kind === "global");
  const directCount = channels.filter((c) => c.kind === "direct").length;
  const directs = channels
    .filter((c) => c.kind === "direct")
    .filter((c) => !term || titleOf(c, t).toLocaleLowerCase().includes(term));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative shrink-0 p-2">
        <Icon
          name="search"
          className="pointer-events-none absolute top-1/2 left-4 size-4 -translate-y-1/2 text-muted"
        />
        <input
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          placeholder={t("social.search")}
          aria-label={t("social.search")}
          spellCheck={false}
          autoComplete="off"
          className="h-9 w-full rounded-lg border border-border bg-surface-2 pr-3 pl-8 text-sm text-text outline-none placeholder:text-muted focus:border-accent"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {term && (users.length > 0 || searching) ? (
          <>
            <SectionLabel>{t("social.players")}</SectionLabel>
            {searching && users.length === 0 ? (
              <p role="status" className="px-3 py-2 text-xs text-muted">
                {t("common.loading")}
              </p>
            ) : null}
            {users.map((user) => (
              <button
                key={user.uuid}
                type="button"
                className="flex w-full cursor-pointer items-center gap-3 px-3 py-2 text-left hover:bg-surface-2"
                onClick={() => onOpenProfile(user)}
              >
                <ChatAvatar user={user} size="sm" />
                <span className="min-w-0 flex-1 truncate text-sm">
                  {user.name}
                </span>
                <span className="shrink-0 text-[11px] text-muted">
                  {t(`social.status.${user.status ?? "hidden"}`)}
                </span>
              </button>
            ))}
          </>
        ) : null}
        <SectionLabel>{t("social.sections.global")}</SectionLabel>
        {global.map((channel) => (
          <ChannelRow
            key={channel.id}
            channel={channel}
            active={channel.id === selected}
            onSelect={onSelect}
          />
        ))}
        <SectionLabel>{t("social.sections.direct")}</SectionLabel>
        {directs.length > 0 ? (
          directs.map((channel) => (
            <ChannelRow
              key={channel.id}
              channel={channel}
              active={channel.id === selected}
              onSelect={onSelect}
            />
          ))
        ) : directCount === 0 ? (
          <p className="px-3 py-2 text-xs text-muted">{t("social.noChats")}</p>
        ) : null}
      </div>
    </div>
  );
}

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <p className="px-3 pt-2 pb-1 text-[11px] font-semibold tracking-wide text-muted uppercase">
      {children}
    </p>
  );
}

function titleOf(
  channel: SocialChannel,
  t: (key: string) => string,
): string {
  if (channel.kind === "global") return t("social.global");
  return channel.peer?.name ?? t("social.directChat");
}

function ChannelRow({
  channel,
  active,
  onSelect,
}: {
  channel: SocialChannel;
  active: boolean;
  onSelect: (channel: SocialChannel) => void;
}) {
  const { t, i18n } = useTranslation();
  const last = channel.lastMessage;
  const preview = previewOf(last);
  return (
    <button
      type="button"
      aria-current={active ? "true" : undefined}
      className={`flex w-full cursor-pointer items-center gap-3 px-3 py-2 text-left transition-colors ${
        active ? "bg-accent/10" : "hover:bg-surface-2"
      }`}
      onClick={() => onSelect(channel)}
    >
      <ChatAvatar
        user={channel.peer}
        icon={channel.kind === "global" ? "users" : "user"}
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-medium">
            {titleOf(channel, t)}
          </span>
          {last ? (
            <time
              className="shrink-0 text-[11px] text-muted"
              dateTime={last.createdAt}
            >
              {clockLabel(last.createdAt, i18n.language)}
            </time>
          ) : null}
        </span>
        <span className="mt-0.5 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs text-muted">
            {!preview
              ? t("social.noMessages")
              : preview.kind === "text"
                ? preview.text
                : t(preview.key, preview.params)}
          </span>
          {channel.unread > 0 ? (
            <Badge tone="accent">
              <span aria-label={t("social.unreadBadge", { n: channel.unread })}>
                {channel.unread}
              </span>
            </Badge>
          ) : null}
        </span>
      </span>
    </button>
  );
}
