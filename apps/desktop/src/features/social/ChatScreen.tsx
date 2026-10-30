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
import { api, isMock, unwrap } from "../../api/client";
import { createTaskChannel } from "../../api/channel";
import { pickFile } from "../../api/dialog";
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
  canModerate,
  isAdmin,
  isGuest,
  privacyOptions,
} from "../../lib/social";
import { canEditProfile } from "../../lib/socialProfile";
import { useNavStore } from "../../stores/navStore";
import { useSocialStore } from "../../stores/socialStore";
import { useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";
import { AdminPanel } from "./admin/AdminPanel";
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
import { ProfileCard } from "./ProfileCard";
import { ProfileEditor } from "./ProfileEditor";

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

const SEARCH_DEBOUNCE_MS = 250;

export function ChatScreen() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const session = useSocialStore();
  const self = session.self;
  const wide = useMediaQuery("(min-width: 768px)");
  const [text, setText] = useState("");
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [profile, setProfile] = useState<SocialUser | null>(null);
  const [editor, setEditor] = useState(false);
  const [tab, setTab] = useState("chat");
  const [pane, setPane] = useState<"list" | "chat">("list");
  const [guestLogin, setGuestLogin] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskStore((s) => (taskId ? s.tasks[taskId] : undefined));
  const room = useRoomState();
  useEffect(() => {
    setText("");
  }, [session.selected]);
  useEffect(() => {
    const value = search.trim();
    if (!value) {
      setQuery("");
      return;
    }
    const timer = setTimeout(() => setQuery(value), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    setProfile(null);
    setEditor(false);
    setQuery("");
    setSearch("");
    setText("");
    setTaskId(null);
    setTab("chat");
    setPane("list");
  }, [self?.uuid]);
  const enabled = !!self;
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
  const friends = useQuery({
    queryKey: ["social", "friends"],
    queryFn: async () => unwrap(await api.socialFriends()),
    enabled,
  });
  const privacy = useQuery({
    queryKey: ["social", "privacy"],
    queryFn: async () => unwrap(await api.socialPrivacy()),
    enabled,
  });
  const users = useQuery({
    queryKey: ["social", "search", query],
    queryFn: async () => unwrap(await api.socialSearchUsers(query)),
    enabled: enabled && !!query,
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
  const guest = isGuest(self);
  const selected = channels.data?.find((c) => c.id === session.selected);
  const directChat = selected?.kind === "direct";
  const peer = directChat ? (selected?.peer ?? null) : null;
  const blocked = peer?.isBlocked === true || (!!peer && session.blocked.includes(peer.uuid));
  const title = directChat ? (peer?.name ?? t("social.directChat")) : t("social.global");
  const canInvitePeer = !!room.data && !!peer && peer.canInvite && !blocked;
  const latest = messages.at(-1);
  const markRead = () => {
    if (latest)
      run(async () => {
        unwrap(await api.socialMarkRead(session.selected, latest.id));
      });
  };
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
      const dest = isMock
        ? `C:/Downloads/${attachment.fileName}`
        : await (
            await import("@tauri-apps/plugin-dialog")
          ).save({ defaultPath: attachment.fileName });
      if (dest)
        await transfer((ch) =>
          api.socialSaveAttachment(attachment.id, dest, ch),
        );
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
  return (
    <div className="flex h-[calc(100dvh-6.5rem)] min-h-[30rem] flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="mr-auto text-xl font-semibold">{t("social.title")}</h2>
        <OnlineBadge enabled={session.connected} />
        {guest ? <Badge tone="info">{t("social.guestBadge")}</Badge> : null}
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
      {tab === "chat" ? (
        <div className="min-h-0 flex-1 overflow-hidden rounded-xl border border-border bg-panel">
          <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[320px_minmax(0,1fr)]">
            <aside
              aria-label={t("social.channels")}
              className={`min-h-0 border-border md:border-r ${
                wide || pane === "list" ? "flex flex-col" : "hidden"
              }`}
            >
              <ChatChannelList
                channels={
                  guest
                    ? (channels.data ?? []).filter((c) => c.kind === "global")
                    : channels.data ?? []
                }
                selected={session.selected}
                onSelect={pickChannel}
                users={query ? (users.data ?? []) : []}
                searching={!!query && users.isFetching}
                search={search}
                onSearch={setSearch}
                onOpenProfile={openProfile}
                showDirect={!guest}
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
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={t("social.markRead")}
                    title={t("social.markRead")}
                    disabled={!latest || action.isPending}
                    onClick={markRead}
                    icon={<Icon name="check" />}
                  />
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
                  renderRow={renderRow}
                  empty={t(history.isPending ? "common.loading" : "social.empty")}
                />
              </div>
              {guest ? (
                <p
                  role="status"
                  className="shrink-0 border-t border-border bg-panel px-3 py-3 text-center text-sm text-muted"
                >
                  {t("social.guestReadOnly")}
                </p>
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
      ) : null}
      {tab === "friends" ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <VirtualList
            items={friends.data ?? []}
            rowHeight={80}
            height={480}
            getKey={(f) => f.user.uuid}
            ariaLabel={t("social.tabs.friends")}
            renderRow={(friend) => (
              <div className="flex items-center gap-2 rounded-lg border border-border bg-panel p-3">
                <button
                  className="mr-auto"
                  onClick={() => openProfile(friend.user)}
                >
                  {friend.user.name} · {t(`social.friend.${friend.state}`)}
                </button>
                {friend.state === "incoming" ? (
                  <Button
                    size="sm"
                    onClick={() =>
                      run(async () => {
                        unwrap(await api.socialAddFriend(friend.user.uuid));
                      })
                    }
                  >
                    {t("social.accept")}
                  </Button>
                ) : null}
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
            )}
          />
        </div>
      ) : null}
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
            else unwrap(await api.socialAddFriend(user.uuid));
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
