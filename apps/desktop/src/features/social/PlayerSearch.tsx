import { useTranslation } from "react-i18next";

import type { SocialUser } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { displayNameOf } from "../../lib/socialProfile";
import { ChatAvatar } from "./ChatAvatar";

export interface PlayerSearchProps {
  search: string;
  onSearch: (value: string) => void;
  /** Results of the debounced query; empty while the field is blank. */
  users: SocialUser[];
  searching: boolean;
  onOpenProfile: (user: SocialUser) => void;
  onAddFriend: (user: SocialUser) => void;
}

/** Player lookup shared by the friends tab: field plus result rows with actions. */
export function PlayerSearch({
  search,
  onSearch,
  users,
  searching,
  onOpenProfile,
  onAddFriend,
}: PlayerSearchProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="relative">
        <Icon
          name="search"
          className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted"
        />
        <input
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          placeholder={t("social.searchPlayers")}
          aria-label={t("social.searchPlayers")}
          spellCheck={false}
          autoComplete="off"
          className="h-9 w-full rounded-lg border border-border bg-surface-2 pr-3 pl-9 text-sm text-text outline-none placeholder:text-muted focus:border-accent"
        />
      </div>
      {searching ? (
        <p role="status" className="px-1 text-xs text-muted">
          {t("common.loading")}
        </p>
      ) : null}
      {!searching && users.length === 0 && search.trim() ? (
        <p className="px-1 text-xs text-muted">{t("social.noPlayers")}</p>
      ) : null}
      {users.map((user) => (
        <div
          key={user.uuid}
          data-testid="player-search-result"
          className="flex items-center gap-2 rounded-lg border border-border bg-panel p-2 pl-3"
        >
          <ChatAvatar user={user} size="sm" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">
              {displayNameOf(user)}
            </span>
            <span className="block truncate text-xs text-muted">
              {t(`social.status.${user.status ?? "hidden"}`)}
            </span>
          </span>
          <div className="flex shrink-0 gap-1">
            <Button
              size="sm"
              disabled={user.isFriend}
              onClick={() => onAddFriend(user)}
            >
              {t(user.isFriend ? "social.alreadyFriends" : "social.addFriend")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onOpenProfile(user)}
            >
              {t("social.openProfile")}
            </Button>
          </div>
        </div>
      ))}
    </div>
  );
}
