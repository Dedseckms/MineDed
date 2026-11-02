import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AdminAccount, SocialUser } from "../../../bindings";
import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { CopyButton } from "../../../components/ui/CopyButton";
import { Badge, ErrorMessage } from "../../../components/ui/feedback";
import { TextInput } from "../../../components/ui/form";
import { displayNameOf } from "../../../lib/socialProfile";
import { ChatAvatar } from "../ChatAvatar";
import { AdminSplit, ListRow, useSocialAction } from "./shared";

const MUTE_OPTIONS = [3600, 86400] as const;

/** Case-insensitive match over the displayed name, nickname and UUID. */
export function matchesAccount(
  account: AdminAccount,
  filter: string,
): boolean {
  const q = filter.trim().toLowerCase();
  if (!q) return true;
  return (
    account.name.toLowerCase().includes(q) ||
    (account.displayName ?? "").toLowerCase().includes(q) ||
    account.uuid.toLowerCase().startsWith(q)
  );
}

/** Every account the launcher knows, plus the player search for chat actions. */
export function UsersTab() {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<SocialUser | null>(null);
  const accounts = useQuery({
    queryKey: ["social", "adminAccounts"],
    queryFn: async () => unwrap(await api.socialAdminAccounts()),
  });
  const users = useQuery({
    queryKey: ["social", "adminSearch", query],
    queryFn: async () => unwrap(await api.socialSearchUsers(query)),
    enabled: !!query,
  });
  const action = useSocialAction();
  const submitSearch = (e: FormEvent) => {
    e.preventDefault();
    setQuery(search.trim());
  };
  const known = accounts.data ?? [];
  const visible = known.filter((a) => matchesAccount(a, search));
  const when = (at: string) => new Date(at).toLocaleString(i18n.language);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <ErrorMessage error={action.error ?? accounts.error ?? users.error} />
      <AdminSplit
        list={
          <>
            <div className="flex shrink-0 flex-col gap-2 border-b border-border p-2">
              <label className="flex flex-col gap-1">
                <span className="text-xs text-muted">
                  {t("social.admin.accounts.filter")}
                </span>
                <TextInput
                  aria-label={t("social.admin.accounts.filter")}
                  placeholder={t("social.admin.accounts.filterPlaceholder")}
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                />
              </label>
              <form className="flex gap-2" onSubmit={submitSearch}>
                <TextInput
                  aria-label={t("social.search")}
                  placeholder={t("social.search")}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
                <Button type="submit" size="md">
                  {t("common.search")}
                </Button>
              </form>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {query ? (
                <section
                  aria-label={t("social.admin.searchResults")}
                  className="border-b border-border"
                >
                  {!users.isFetching && (users.data?.length ?? 0) === 0 ? (
                    <p className="px-3 py-2 text-xs text-muted">
                      {t("social.admin.noUsers")}
                    </p>
                  ) : null}
                  {(users.data ?? []).map((user) => (
                    <ListRow
                      key={`search-${user.uuid}`}
                      active={selected?.uuid === user.uuid}
                      onClick={() => setSelected(user)}
                    >
                      <span className="flex w-full items-center gap-2">
                        <ChatAvatar user={user} size="sm" />
                        <span className="min-w-0 flex-1 truncate text-sm font-medium">
                          {displayNameOf(user)}
                        </span>
                        <Badge
                          tone={user.role === "admin" ? "accent" : "neutral"}
                        >
                          {t(`social.role.${user.role}`)}
                        </Badge>
                      </span>
                      <span className="truncate text-xs text-muted">
                        {user.name}
                      </span>
                    </ListRow>
                  ))}
                </section>
              ) : null}
              <section aria-label={t("social.admin.accounts.title")}>
                {visible.length === 0 && !accounts.isFetching ? (
                  <p className="px-3 py-4 text-sm text-muted">
                    {t("social.admin.accounts.empty")}
                  </p>
                ) : null}
                {visible.map((account) => (
                  <ListRow
                    key={account.uuid}
                    active={
                      !query &&
                      selected?.uuid.toLowerCase() === account.uuid.toLowerCase()
                    }
                    onClick={() => {
                      setQuery("");
                      const live = users.data?.find(
                        (u) =>
                          u.uuid.toLowerCase() === account.uuid.toLowerCase(),
                      );
                      setSelected(
                        live ?? {
                          uuid: account.uuid,
                          name: account.name,
                          displayName: account.displayName ?? null,
                          skinUrl: null,
                          status: "offline",
                          role: account.guest ? "guest" : "user",
                          guest: account.guest,
                          isFriend: false,
                          canMessage: false,
                          canInvite: false,
                        },
                      );
                    }}
                  >
                    <span className="flex w-full items-center gap-2">
                      <span className="min-w-0 flex-1 truncate text-sm font-medium">
                        {account.displayName || account.name}
                      </span>
                      <Badge tone={account.guest ? "warning" : "accent"}>
                        {t(
                          account.guest
                            ? "social.profile.guest"
                            : "social.admin.accounts.real",
                        )}
                      </Badge>
                    </span>
                    <span className="truncate text-xs text-muted">
                      {t("social.admin.accounts.firstSeen", {
                        date: when(account.createdAt),
                      })}
                    </span>
                    {account.invite ? (
                      <span className="truncate text-xs text-accent">
                        {t("social.admin.accounts.invited")}
                      </span>
                    ) : null}
                  </ListRow>
                ))}
              </section>
            </div>
          </>
        }
        details={
          selected ? (
            <div className="flex flex-col gap-4">
              <header className="flex items-center gap-3">
                <ChatAvatar user={selected} size="lg" />
                <div className="min-w-0">
                  <p className="truncate text-lg font-semibold">
                    {displayNameOf(selected)}
                  </p>
                  <p className="truncate text-xs text-muted">@{selected.name}</p>
                  <div className="mt-1 flex flex-wrap gap-1">
                    <Badge
                      tone={selected.role === "admin" ? "accent" : "neutral"}
                    >
                      {t(`social.role.${selected.role}`)}
                    </Badge>
                    <Badge>
                      {t(`social.status.${selected.status ?? "hidden"}`)}
                    </Badge>
                    {selected.guest ? (
                      <Badge tone="warning">{t("social.profile.guest")}</Badge>
                    ) : null}
                  </div>
                </div>
              </header>
              <div className="space-y-1">
                <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  {t("social.admin.userUuid")}
                </p>
                <p className="font-mono text-xs break-all">{selected.uuid}</p>
              </div>
              <AccountInvite
                account={known.find(
                  (a) =>
                    a.uuid.toLowerCase() === selected.uuid.toLowerCase(),
                )}
              />
              <section className="space-y-2">
                <p className="text-sm font-medium">
                  {t("social.admin.roleSection")}
                </p>
                <div className="flex flex-wrap gap-2">
                  {(["user", "moderator"] as const)
                    .filter((role) => role !== selected.role)
                    .map((role) => (
                      <Button
                        key={role}
                        size="sm"
                        variant="secondary"
                        disabled={action.isPending}
                        onClick={() =>
                          action.mutate(async () => {
                            unwrap(
                              await api.socialSetRole(selected.uuid, role),
                            );
                          })
                        }
                      >
                        {t(`social.setRole.${role}`)}
                      </Button>
                    ))}
                </div>
              </section>
              <section className="space-y-2">
                <p className="text-sm font-medium">
                  {t("social.admin.sanctions")}
                </p>
                <div className="flex flex-wrap gap-2">
                  {MUTE_OPTIONS.map((seconds) => (
                    <Button
                      key={seconds}
                      size="sm"
                      disabled={action.isPending}
                      onClick={() =>
                        action.mutate(async () => {
                          unwrap(
                            await api.socialModerateUser(
                              selected.uuid,
                              "mute",
                              seconds,
                            ),
                          );
                        })
                      }
                    >
                      {t("social.mute", { hours: seconds / 3600 })}
                    </Button>
                  ))}
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={action.isPending}
                    onClick={() =>
                      action.mutate(async () => {
                        unwrap(
                          await api.socialModerateUser(
                            selected.uuid,
                            "unmute",
                            null,
                          ),
                        );
                      })
                    }
                  >
                    {t("social.unmute")}
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={action.isPending}
                    onClick={() =>
                      action.mutate(async () => {
                        unwrap(
                          await api.socialModerateUser(
                            selected.uuid,
                            "ban",
                            null,
                          ),
                        );
                      })
                    }
                  >
                    {t("social.ban")}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={action.isPending}
                    onClick={() =>
                      action.mutate(async () => {
                        unwrap(
                          await api.socialModerateUser(
                            selected.uuid,
                            "unban",
                            null,
                          ),
                        );
                      })
                    }
                  >
                    {t("social.unban")}
                  </Button>
                </div>
              </section>
              <section className="space-y-2">
                <p className="text-sm font-medium">
                  {t("social.admin.profileSection")}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" variant="secondary" disabled>
                    {t("social.admin.resetProfile")}
                  </Button>
                  <Button size="sm" variant="secondary" disabled>
                    {t("social.admin.resetAvatar")}
                  </Button>
                </div>
                <p className="text-xs text-muted">
                  {t("social.admin.resetPending")}
                </p>
              </section>
            </div>
          ) : (
            <p className="text-sm text-muted">{t("social.admin.selectUser")}</p>
          )
        }
      />
    </div>
  );
}

/** Invite already issued for this account (from the Drasl request flow). */
function AccountInvite({ account }: { account: AdminAccount | undefined }) {
  const { t } = useTranslation();
  if (!account?.invite && !account?.inviteUrl) return null;
  return (
    <section className="space-y-1">
      <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
        {t("social.admin.accounts.invite")}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 font-mono text-xs break-all">
          {account.inviteUrl ?? account.invite}
        </span>
        <CopyButton value={account.inviteUrl ?? account.invite ?? ""} />
        {account.inviteUrl && account.invite ? (
          <CopyButton
            value={account.invite}
            label={t("social.admin.accounts.inviteCode")}
          />
        ) : null}
      </div>
    </section>
  );
}
