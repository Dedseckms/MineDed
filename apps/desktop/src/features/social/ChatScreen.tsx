import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { type CSSProperties, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  SocialAttachment,
  SocialChannel,
  SocialContent,
  SocialMessage,
  SocialPrivacy,
  SocialUser,
} from "../../bindings";
import { api, unwrap } from "../../api/client";
import { createTaskChannel } from "../../api/channel";
import { pickFile, pickSavePath } from "../../api/dialog";
import { useRoomState } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { VirtualList } from "../../components/ui/VirtualList";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { Select } from "../../components/ui/form";
import { useMediaQuery } from "../../hooks/useAppearance";
import { applySocialEvent } from "../../hooks/useSocialSync";
import {
  hasRequestedAccount,
  markAccountRequested,
} from "../../lib/accountRequest";
import {
  canModerate,
  isAdmin,
  isGuest,
  privacyOptions,
} from "../../lib/social";
import { canEditProfile, displayNameOf } from "../../lib/socialProfile";
import { useNavStore } from "../../stores/navStore";
import { useSocialStore } from "../../stores/socialStore";
import { useToastStore } from "../../stores/toastStore";
import { useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";
import { AdminPanel } from "./admin/AdminPanel";
import { AccountRequestDialog } from "./AccountRequestDialog";
import { ChatAvatar } from "./ChatAvatar";
import {
  type ChatMessageActions,
  DaySeparator,
  MessageGroupView,
} from "./ChatBubble";
import { ChatChannelList } from "./ChatChannelList";
import { ChatComposer } from "./ChatComposer";
import { ChatTimeline } from "./ChatTimeline";
import { type TimelineRow, buildTimeline, chronological } from "./chat";
import { SharePack } from "./PackShare";
import { GuestLoginDialog } from "./GuestLoginDialog";
import { OnlineBadge } from "./OnlineBadge";
import { PlayerSearch } from "./PlayerSearch";
import { ProfileCard } from "./ProfileCard";
import { ProfileEditor } from "./ProfileEditor";
import { saveFileName } from "./saveFileName";
import { useAutoMarkRead } from "./useAutoMarkRead";
import { useDebouncedSearch } from "./useDebouncedSearch";

const privacyFields: (keyof SocialPrivacy)[] = [
  "whoCanMessage",
  "whoCanInvite",
  "whoSeesStatus",
  "whoCanAddFriend",
];

/** Soft Telegram-like pattern built from theme tokens (no images, no blur). */
const CHAT_PATTERN: CSSProperties = {
  backgroundImage:
    "radial-gradient(circle at center, color-mix(in srgb, var(--color-border) 55%, transparent) 1px, transparent 1px), linear-gradient(180deg, color-mix(in srgb, var(--color-accent) 5%, transparent), transparent 60%)",
  backgroundSize: "22px 22px, 100% 100%",
};

export function ChatScreen() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const session = useSocialStore();
  const self = session.self;
  const wide = useMediaQuery("(min-width: 768px)");
  const [text, setText] = useState("");
  const chatSearch = useDebouncedSearch();
  const friendSearch = useDebouncedSearch();
  const [profile, setProfile] = useState<SocialUser | null>(null);
  const [editor, setEditor] = useState(false);
  const [tab, setTab] = useState("chat");
  const [pane, setPane] = useState<"list" | "chat">("list");
  const [guestLogin, setGuestLogin] = useState(false);
  const [accountDialog, setAccountDialog] = useState(false);
  const [accountRequested, setAccountRequested] = useState(() =>
    hasRequestedAccount(self?.uuid ?? ""),
  );
  useEffect(() => {
    setAccountRequested(hasRequestedAccount(self?.uuid ?? ""));
    setAccountDialog(false);
  }, [self?.uuid]);
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskStore((s) => (taskId ? s.tasks[taskId] : undefined));
  const room = useRoomState();
  useEffect(() => {
    setText("");
  }, [session.selected]);
  useEffect(() => {
    setProfile(null);
    setEditor(false);
    chatSearch.reset();
    friendSearch.reset();
    setText("");
    setTaskId(null);
    setTab("chat");
    setPane("list");
  }, [self?.uuid]);
  useEffect(() => {
    if (tab !== "friends") {
      friendSearch.reset();
      return;
    }
    if (!chatSearch.query) return;
    chatSearch.reset();
  }, [tab]);
  const enabled = !!self;
  const guest = !!self && isGuest(self);
  const accountEnabled = enabled && !guest;
  const channels = useQuery({
    queryKey: ["social", "channels"],
    queryFn: async () => unwrap(await api.socialChannels()),
    enabled,
    refetchInterval: 30000,
  });
  const history = useInfiniteQuery({
    queryKey: ["social", "history", session.selected],
    queryFn: async ({ pageParam }) =>
      unwrap(await api.socialHistory(session.selected, pageParam)),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.before ?? undefined,
    enabled,
  });
  // Friends and privacy are account features; a guest would only get permissionDenied.
  const friends = useQuery({
    queryKey: ["social", "friends"],
    queryFn: async () => unwrap(await api.socialFriends()),
    enabled: accountEnabled,
  });
  const privacy = useQuery({
    queryKey: ["social", "privacy"],
    queryFn: async () => unwrap(await api.socialPrivacy()),
    enabled: accountEnabled,
  });
  const users = useQuery({
    queryKey: ["social", "search", friendSearch.query],
    queryFn: async () => unwrap(await api.socialSearchUsers(friendSearch.query)),
    enabled: accountEnabled && !!friendSearch.query,
  });
  const action = useMutation({
    mutationFn: async (run: () => Promise<unknown>) => run(),
    onSuccess: () =>
      qc.invalidateQueries({
        queryKey: ["social"],
        predicate: (query) =>
          query.queryKey[1] !== "history" && query.queryKey[1] !== "preview",
      }),
  });
  const run = (fn: () => Promise<unknown>) => action.mutate(fn);
  const messages = useMemo(
    () => chronological(history.data?.pages ?? []),
    [history.data],
  );
  const rows = useMemo(
    () => buildTimeline(messages, self?.uuid ?? ""),
    [messages, self?.uuid],
  );
  const latestId = useMemo(() => messages.at(-1)?.id ?? null, [messages]);
  const [pinned, setPinned] = useState(true);
  const markRead = (channel: string, messageId: string) => {
    void (async () => {
      try {
        unwrap(await api.socialMarkRead(channel, messageId));
      } catch {
        return;
      }
      void qc.invalidateQueries({ queryKey: ["social", "channels"] });
    })();
  };
  useAutoMarkRead({
    channel: session.selected,
    latestId,
    pinned,
    markRead,
    enabled: !!self,
  });
  useEffect(() => {
    session.set({
      unread: channels.data?.reduce((n, c) => n + c.unread, 0) ?? 0,
    });
  }, [channels.data]);
  const openProfile = (user: SocialUser) =>
    run(async () => setProfile(unwrap(await api.socialProfile(user.uuid))));
  const direct = (user: SocialUser) =>
    run(async () => {
      const channel = unwrap(await api.socialOpenDirect(user.uuid));
      session.set({ selected: channel.id });
      setTab("chat");
      setPane("chat");
      setProfile(null);
    });
  const transfer = async (
    start: (
      ch: ReturnType<typeof createTaskChannel>,
    ) => ReturnType<typeof api.socialSendFile>,
  ) => {
    const ch = createTaskChannel((ev) => {
      useTaskStore.getState().applyEvent(ev);
      if (ev.type === "finished")
        void qc.invalidateQueries({ queryKey: ["social"] });
    });
    setTaskId(unwrap(await start(ch)));
  };
  const send = (content: SocialContent) =>
    run(async () => {
      const message = unwrap(await api.socialSend(session.selected, content));
      applySocialEvent(qc, { type: "message", message });
      setText("");
    });
  if (!self)
    return (
      <div className="space-y-4 rounded-xl border border-border bg-panel p-6">
        <h2 className="text-xl font-semibold">{t("social.title")}</h2>
        <p>{t("social.login")}</p>
        <ErrorMessage error={session.error} />
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => useNavStore.getState().go("accounts")}>
            {t("nav.accounts")}
          </Button>
          <Button variant="secondary" onClick={() => setGuestLogin(true)}>
            {t("social.guestLogin")}
          </Button>
        </div>
        <GuestLoginDialog
          open={guestLogin}
          onClose={() => setGuestLogin(false)}
          onLogin={(user) =>
            session.set({ self: user, connected: true, error: null })
          }
        />
      </div>
    );
  const guestMode = isGuest(self);
  const selected = channels.data?.find((c) => c.id === session.selected);
  const directChat = selected?.kind === "direct";
  const peer = directChat ? (selected?.peer ?? null) : null;
  const blocked = peer?.isBlocked === true || (!!peer && session.blocked.includes(peer.uuid));
  const title = directChat ? (peer?.name ?? t("social.directChat")) : t("social.global");
  const canInvitePeer = !!room.data && !!peer && peer.canInvite && !blocked;
  const invite = (user: SocialUser) => {
    const code = room.data?.code;
    if (!code) return;
    run(async () => {
      const channel = unwrap(await api.socialOpenDirect(user.uuid));
      unwrap(
        await api.socialSend(channel.id, {
          type: "roomInvite",
          code,
          link: `mineded://join/${code}`,
        }),
      );
    });
  };
  const saveAttachment = (attachment: SocialAttachment) =>
    run(async () => {
      const name = saveFileName(attachment);
      const ext = name.slice(name.lastIndexOf(".") + 1);
      const dest = await pickSavePath({
        defaultPath: name,
        extensions: name.includes(".") ? [ext] : undefined,
      });
      if (dest)
        await transfer((ch) =>
          api.socialSaveAttachment(attachment.id, dest, ch),
        );
    });
  const addFriend = (user: SocialUser) =>
    run(async () => {
      unwrap(await api.socialAddFriend(user.uuid));
      useToastStore.getState().push({ tone: "info", messageKey: "social.friendRequestSent" });
    });
  const actions: ChatMessageActions = {
    selfUuid: self.uuid,
    canModerate: canModerate(self.role),
    showAuthors: !directChat,
    onOpenProfile: openProfile,
    onDelete: (message: SocialMessage) =>
      run(async () => {
        unwrap(await api.socialDeleteMessage(message.id));
      }),
    onReport: (message: SocialMessage) =>
      run(async () => {
        unwrap(await api.socialReport(message.id, null));
        useToastStore.getState().push({ tone: "info", messageKey: "social.reportSent" });
      }),
    onModerateDelete: (message: SocialMessage) =>
      run(async () => {
        unwrap(await api.socialModerateDelete(message.id));
      }),
    onSave: saveAttachment,
  };
  const renderRow = (row: TimelineRow) =>
    row.kind === "day" ? (
      <DaySeparator at={row.at} />
    ) : (
      <MessageGroupView group={row.group} actions={actions} />
    );
  const pickChannel = (channel: SocialChannel) => {
    session.set({ selected: channel.id });
    setPane("chat");
  };
  const chatLayout = tab === "chat";
  const showIncoming = !!friendSearch.search.trim();
  const friendResults = friendSearch.query ? (users.data ?? []) : [];
  const incoming = (friends.data ?? []).filter((f) => f.state === "incoming");
  return (
    <div className="flex h-[calc(100dvh-6.5rem)] min-h-[30rem] flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="mr-auto text-xl font-semibold">{t("social.title")}</h2>
        <OnlineBadge enabled={session.connected} />
        {guestMode ? <Badge tone="info">{t("social.guestBadge")}</Badge> : null}
        <Badge tone={session.connected ? "accent" : "warning"}>
          {t(session.connected ? "social.connected" : "social.reconnecting")}
        </Badge>
        {canEditProfile(self) ? (
          <Button
            size="sm"
            variant="ghost"
            icon={<Icon name="user" />}
            onClick={() => setEditor(true)}
          >
            {t("social.profile.self")}
          </Button>
        ) : null}
        <div className="flex gap-1">
          {[
            "chat",
            ...(isGuest(self) ? [] : ["friends", "privacy"]),
            ...(isAdmin(self.role) ? ["admin"] : []),
          ].map((name) => (
            <Button
              key={name}
              size="sm"
              variant={tab === name ? "primary" : "ghost"}
              aria-pressed={tab === name}
              onClick={() => setTab(name)}
            >
              {t(`social.tabs.${name}`)}
            </Button>
          ))}
        </div>
      </div>
      <ErrorMessage
        error={
          session.error ??
          action.error ??
          channels.error ??
          history.error ??
          friends.error ??
          privacy.error ??
          users.error
        }
      />
      {task ? <TaskProgress task={task} compact /> : null}
      <div
        className={`rounded-xl border border-border bg-panel ${
          chatLayout ? "min-h-0 flex-1 overflow-hidden" : "hidden"
        }`}
      >
        <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[320px_minmax(0,1fr)]">
          <aside
            aria-label={t("social.channels")}
            className={`min-h-0 border-border md:border-r ${
              wide || pane === "list" ? "flex flex-col" : "hidden"
            }`}
          >
            <ChatChannelList
              channels={
                guestMode
                  ? (channels.data ?? []).filter((c) => c.kind === "global")
                  : channels.data ?? []
              }
              selected={session.selected}
              onSelect={pickChannel}
              search={chatSearch.search}
              onSearch={chatSearch.setSearch}
              showDirect={!guestMode}
            />
          </aside>
          <section
            aria-label={title}
            className={`min-h-0 flex-col ${
              wide || pane === "chat" ? "flex" : "hidden"
            }`}
          >
            <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-2">
              {wide ? null : (
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={t("social.back")}
                  title={t("social.back")}
                  onClick={() => setPane("list")}
                  icon={<Icon name="back" />}
                />
              )}
              <ChatAvatar
                user={peer}
                icon={directChat ? "user" : "users"}
                size="sm"
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-semibold">{title}</p>
                <p className="truncate text-[11px] text-muted">
                  {directChat
                    ? t(`social.status.${peer?.status ?? "hidden"}`)
                    : t("social.publicChannel")}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {directChat ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={t("social.profile.title")}
                    title={t("social.profile.title")}
                    onClick={() => {
                      if (peer) openProfile(peer);
                    }}
                    icon={<Icon name="user" />}
                  />
                ) : null}
                {directChat ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={t("social.invite")}
                    title={t("social.invite")}
                    disabled={!canInvitePeer}
                    onClick={() => {
                      if (peer) invite(peer);
                    }}
                    icon={<Icon name="link" />}
                  />
                ) : null}
              </div>
            </header>
            <div
              className="relative flex min-h-0 flex-1 flex-col"
              style={CHAT_PATTERN}
            >
              <ChatTimeline
                rows={rows}
                channel={session.selected}
                hasOlder={!!history.hasNextPage}
                loadingOlder={history.isFetchingNextPage}
                onLoadOlder={() => void history.fetchNextPage()}
                onAtBottomChange={setPinned}
                renderRow={renderRow}
                empty={t(history.isPending ? "common.loading" : "social.empty")}
              />
            </div>
            {guestMode ? (
              <div
                className="flex shrink-0 flex-wrap items-center justify-center gap-2 border-t border-border bg-panel px-3 py-3"
              >
                <p role="status" className="text-center text-sm text-muted">
                  {t("social.guestReadOnly")}
                </p>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={accountRequested}
                  icon={<Icon name={accountRequested ? "check" : "user"} />}
                  onClick={() => setAccountDialog(true)}
                >
                  {t(
                    accountRequested
                      ? "social.accountRequest.sent"
                      : "social.accountRequest.ask",
                  )}
                </Button>
              </div>
            ) : (
              <ChatComposer
                value={text}
                onChange={setText}
                onSend={() => {
                  const value = text.trim();
                  if (value) send({ type: "text", text: value });
                }}
                onAttach={() =>
                  run(async () => {
                    const path = await pickFile();
                    if (path)
                      await transfer((ch) =>
                        api.socialSendFile(
                          session.selected,
                          path,
                          text.trim() || null,
                          ch,
                        ),
                      );
                  })
                }
                disabled={action.isPending || !session.connected}
                hint={t(
                  directChat ? "social.directHint" : "social.rules",
                )}
                tools={
                  directChat && selected ? (
                    <SharePack
                      key={selected.id}
                      channel={selected.id}
                      disabled={
                        action.isPending ||
                        !session.connected ||
                        !selected.peer?.canInvite ||
                        selected.peer?.isBlocked === true
                      }
                    />
                  ) : null
                }
              />
            )}
          </section>
        </div>
      </div>
      <div className="flex flex-col gap-3" hidden={tab !== "friends"}>
        <div className="min-h-0 flex-1 overflow-y-auto space-y-2 rounded-xl border border-border bg-panel p-3">
          <PlayerSearch
            search={friendSearch.search}
            onSearch={friendSearch.setSearch}
            users={friendResults}
            searching={!!friendSearch.query && users.isFetching}
            onOpenProfile={openProfile}
            onAddFriend={addFriend}
          />
          <div className="flex flex-col" hidden={showIncoming}>
            {incoming.length === 0 ? (
              <p className="px-1 py-2 text-xs text-muted">
                {t("social.noIncoming")}
              </p>
            ) : null}
            <VirtualList
              items={friends.data ?? []}
              rowHeight={112}
              height={480}
              getKey={(f) => f.user.uuid}
              ariaLabel={t("social.tabs.friends")}
              renderRow={(friend) => (
                <div className="flex items-center gap-2 rounded-lg border border-border bg-panel p-3">
                  <ChatAvatar user={friend.user} size="sm" />
                  <button
                    className="mr-auto text-left"
                    onClick={() => openProfile(friend.user)}
                  >
                    <ChatAvatar user={friend.user} size="sm" />
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium">
                        {displayNameOf(friend.user)}
                      </span>
                      <span className="block truncate text-xs text-muted">
                        {t(`social.friend.${friend.state}`)}
                      </span>
                    </span>
                  </button>
                  <div className="flex shrink-0 flex-wrap justify-end gap-1">
                    {friend.state === "incoming" ? (
                      <Button
                        size="sm"
                        onClick={() =>
                          run(async () => {
                            unwrap(await api.socialAddFriend(friend.user.uuid));
                            useToastStore.getState().push({ tone: "info", messageKey: "social.friendRequestAccepted" });
                          })
                        }
                      >
                        {t("social.accept")}
                      </Button>
                    ) : null}
                    {friend.state === "incoming" ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() =>
                          run(async () => {
                            unwrap(await api.socialRemoveFriend(friend.user.uuid));
                            useToastStore.getState().push({ tone: "info", messageKey: "social.friendRequestDeclined" });
                          })
                        }
                      >
                        {t("social.decline")}
                      </Button>
                    ) : null}
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => direct(friend.user)}
                    >
                      {t("social.write")}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => openProfile(friend.user)}
                    >
                      {t("social.openProfile")}
                    </Button>
                    {/* A disabled button swallows tooltips, so the hint sits on the wrapper. */}
                    <span title={t(room.data ? "social.invite" : "social.inviteNoRoom")}>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={!room.data}
                        onClick={() => invite(friend.user)}
                      >
                        {t("social.invite")}
                      </Button>
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        run(async () => {
                          unwrap(await api.socialRemoveFriend(friend.user.uuid));
                        })
                      }
                    >
                      {t(
                        friend.state === "incoming"
                          ? "social.decline"
                          : "common.delete",
                      )}
                    </Button>
                  </div>
                </div>
              )}
            />
          </div>
        </div>
      </div>
      {tab === "privacy" && privacy.data ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="space-y-4 rounded-xl border border-border bg-panel p-4">
            {privacyFields.map((field) => (
              <label key={field} className="block space-y-1">
                <span className="text-sm">{t(`social.privacy.${field}`)}</span>
                <Select
                  value={privacy.data[field]}
                  disabled={action.isPending}
                  onChange={(e) => {
                    const value = privacyOptions(field).find(
                      (v) => v === e.target.value,
                    );
                    if (value && privacy.data) {
                      const next = { ...privacy.data, [field]: value };
                      run(async () => {
                        unwrap(await api.socialSetPrivacy(next));
                      });
                    }
                  }}
                >
                  {privacyOptions(field).map((value) => (
                    <option key={value} value={value}>
                      {t(`social.audience.${value}`)}
                    </option>
                  ))}
                </Select>
              </label>
            ))}
          </div>
        </div>
      ) : null}
      {tab === "admin" && isAdmin(self.role) ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <AdminPanel self={self} />
        </div>
      ) : null}
      {profile ? (
        <Profile
          user={profile}
          self={self}
          close={() => setProfile(null)}
          direct={direct}
          run={run}
          invite={() => invite(profile)}
          canInvite={!!room.data && profile.canInvite}
          edit={() => {
            setProfile(null);
            setEditor(true);
          }}
        />
      ) : null}
      {editor ? (
        <ProfileEditor self={self} onClose={() => setEditor(false)} />
      ) : null}
      {guestMode ? (
        <AccountRequestDialog
          open={accountDialog}
          onClose={() => setAccountDialog(false)}
          requested={accountRequested}
          onSent={() => {
            markAccountRequested(self.uuid);
            setAccountRequested(true);
          }}
        />
      ) : null}
    </div>
  );
}

function Profile({
  user: initialUser,
  self,
  close,
  direct,
  run,
  invite,
  canInvite,
  edit,
}: {
  user: SocialUser;
  self: SocialUser;
  close: () => void;
  direct: (user: SocialUser) => void;
  run: (fn: () => Promise<unknown>) => void;
  invite: () => void;
  canInvite: boolean;
  edit: () => void;
}) {
  const { t } = useTranslation();
  const refreshed = useQuery({
    queryKey: ["social", "profile", initialUser.uuid],
    queryFn: async () => unwrap(await api.socialProfile(initialUser.uuid)),
  });
  const user = refreshed.data ?? initialUser;
  const locallyBlocked = useSocialStore((s) => s.blocked.includes(user.uuid));
  const blocked = user.isBlocked === true || locallyBlocked;
  return (
    <Modal open onClose={close} title={t("social.profile.title")}>
      <ProfileCard
        user={user}
        self={self}
        blocked={blocked}
        canInvite={canInvite}
        onEdit={edit}
        onWrite={() => direct(user)}
        onInvite={invite}
        onToggleFriend={() =>
          run(async () => {
            if (user.isFriend) unwrap(await api.socialRemoveFriend(user.uuid));
            else {
              unwrap(await api.socialAddFriend(user.uuid));
              useToastStore.getState().push({ tone: "info", messageKey: "social.friendRequestSent" });
            }
          })
        }
        onToggleBlock={() =>
          run(async () => {
            unwrap(await api.socialSetBlocked(user.uuid, !blocked));
            useSocialStore.getState().block(user.uuid, !blocked);
          })
        }
        guest={isGuest(self)}
      />
    </Modal>
  );
}

