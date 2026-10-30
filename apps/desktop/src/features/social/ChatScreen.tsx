import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  SocialContent,
  SocialMessage,
  SocialPrivacy,
  SocialUser,
} from "../../bindings";
import { api, isMock, unwrap } from "../../api/client";
import { createTaskChannel } from "../../api/channel";
import { pickFile } from "../../api/dialog";
import { useRoomState } from "../../api/queries";
import { Modal } from "../../components/ui/Modal";
import { Button } from "../../components/ui/Button";
import { TextInput, Select } from "../../components/ui/form";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { VirtualList } from "../../components/ui/VirtualList";
import { applySocialEvent } from "../../hooks/useSocialSync";
import { useMotionPrefs } from "../../hooks/useAppearance";
import { canContact, canModerate, privacyOptions } from "../../lib/social";
import { useNavStore } from "../../stores/navStore";
import { useSocialStore } from "../../stores/socialStore";
import { useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";
import { Skin3D, SkinFlat } from "../skins/SkinPreview";
import { AttachmentPreview } from "./AttachmentPreview";
import { ReceivePack, SharePack } from "./PackShare";
import { SocialAdmin } from "./SocialAdmin";

const privacyFields: (keyof SocialPrivacy)[] = [
  "whoCanMessage",
  "whoCanInvite",
  "whoSeesStatus",
  "whoCanAddFriend",
];

export function ChatScreen() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const session = useSocialStore();
  const self = session.self;
  const [text, setText] = useState("");
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [profile, setProfile] = useState<SocialUser | null>(null);
  const [tab, setTab] = useState("chat");
  const [taskId, setTaskId] = useState<string | null>(null);
  const task = useTaskStore((s) => (taskId ? s.tasks[taskId] : undefined));
  const room = useRoomState();
  useEffect(() => {
    setText("");
  }, [session.selected]);
  useEffect(() => {
    setProfile(null);
    setQuery("");
    setSearch("");
    setText("");
    setTaskId(null);
    setTab("chat");
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
  const messages = [
    ...new Map(
      history.data?.pages
        .flatMap((p) => [...p.messages].reverse())
        .map((m) => [m.id, m]),
    ).values(),
  ];
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
        <Button onClick={() => useNavStore.getState().go("accounts")}>
          {t("nav.accounts")}
        </Button>
      </div>
    );
  const selected = channels.data?.find((c) => c.id === session.selected);
  const renderMessage = (message: SocialMessage) => (
    <article className="m-1 h-[188px] overflow-y-auto rounded-lg border border-border bg-panel p-3 text-sm">
      <div className="flex items-center gap-2">
        <button
          className="font-semibold text-accent"
          onClick={() => openProfile(message.author)}
        >
          {message.author.name}
        </button>
        <time className="text-xs text-muted" dateTime={message.createdAt}>
          {new Date(message.createdAt).toLocaleString()}
        </time>
        <Badge>{t(`social.role.${message.author.role}`)}</Badge>
      </div>
      {message.deleted ? (
        <p className="py-4 text-muted">{t("social.deleted")}</p>
      ) : (
        <>
          <MessageContent
            message={message}
            save={(attachment) =>
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
              })
            }
          />
          <div className="mt-2 flex flex-wrap gap-2">
            {message.author.uuid === self?.uuid ? (
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialDeleteMessage(message.id));
                  })
                }
              >
                {t("common.delete")}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialReport(message.id, null));
                  })
                }
              >
                {t("social.report")}
              </Button>
            )}
            {canModerate(self.role) && message.channel === "global" ? (
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialModerateDelete(message.id));
                  })
                }
              >
                {t("social.moderateDelete")}
              </Button>
            ) : null}
          </div>
        </>
      )}
    </article>
  );
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="mr-auto text-xl font-semibold">{t("social.title")}</h2>
        <Badge tone={session.connected ? "accent" : "warning"}>
          {t(session.connected ? "social.connected" : "social.reconnecting")}
        </Badge>
        {[
          "chat",
          "friends",
          "privacy",
          ...(canModerate(self.role) ? ["admin"] : []),
        ].map((name) => (
          <Button
            key={name}
            variant={tab === name ? "primary" : "ghost"}
            onClick={() => setTab(name)}
          >
            {t(`social.tabs.${name}`)}
          </Button>
        ))}
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
      {task ? <TaskProgress task={task} /> : null}
      {tab === "chat" ? (
        <div className="grid gap-4 md:grid-cols-[200px_1fr]">
          <aside className="space-y-2 rounded-xl border border-border bg-panel p-3">
            <form
              onSubmit={(e) => {
                e.preventDefault();
                setQuery(search.trim());
              }}
              className="flex gap-1"
            >
              <TextInput
                aria-label={t("social.search")}
                placeholder={t("social.search")}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
              <Button type="submit" size="sm">
                {t("common.search")}
              </Button>
            </form>
            <VirtualList
              items={users.data ?? []}
              rowHeight={48}
              height={160}
              getKey={(u) => u.uuid}
              ariaLabel={t("social.search")}
              renderRow={(u) => (
                <button
                  className="w-full truncate p-2 text-left hover:bg-surface-2"
                  onClick={() => openProfile(u)}
                >
                  {u.name}
                </button>
              )}
            />
            <VirtualList
              items={channels.data ?? []}
              rowHeight={56}
              height={390}
              getKey={(c) => c.id}
              ariaLabel={t("social.channels")}
              renderRow={(c) => (
                <button
                  className={`flex w-full items-center justify-between rounded-lg p-3 ${c.id === session.selected ? "bg-surface-2 text-accent" : "hover:bg-surface-2"}`}
                  onClick={() => session.set({ selected: c.id })}
                >
                  <span className="truncate">
                    {c.kind === "global" ? t("social.global") : c.peer?.name}
                  </span>
                  {c.unread > 0 ? <Badge>{c.unread}</Badge> : null}
                </button>
              )}
            />
          </aside>
          <section className="min-w-0 space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="font-medium">
                {selected?.peer?.name ?? t("social.global")}
              </h3>
              <Button
                size="sm"
                variant="ghost"
                disabled={!messages[0]}
                onClick={() =>
                  run(async () => {
                    if (messages[0])
                      unwrap(
                        await api.socialMarkRead(
                          session.selected,
                          messages[0].id,
                        ),
                      );
                  })
                }
              >
                {t("social.markRead")}
              </Button>
            </div>
            <p className="text-xs text-muted">
              {t(
                session.selected === "global"
                  ? "social.rules"
                  : "social.directHint",
              )}
            </p>
            {history.isPending ? (
              <p role="status">{t("common.loading")}</p>
            ) : null}
            {messages.length === 0 && !history.isPending ? (
              <p className="p-6 text-muted">{t("social.empty")}</p>
            ) : null}
            <VirtualList
              key={session.selected}
              items={messages}
              rowHeight={196}
              height={430}
              getKey={(m) => m.id}
              ariaLabel={t("social.history")}
              renderRow={renderMessage}
              onEndReached={() => {
                if (history.hasNextPage && !history.isFetchingNextPage)
                  void history.fetchNextPage();
              }}
            />
            {history.hasNextPage ? (
              <Button
                variant="ghost"
                disabled={history.isFetchingNextPage}
                onClick={() => void history.fetchNextPage()}
              >
                {t("social.older")}
              </Button>
            ) : null}
            <form
              className="space-y-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (text.trim()) send({ type: "text", text: text.trim() });
              }}
            >
              <textarea
                aria-label={t("social.message")}
                placeholder={t("social.message")}
                maxLength={4096}
                value={text}
                onChange={(e) => setText(e.target.value)}
                className="min-h-20 w-full resize-y rounded-lg border border-border bg-surface-2 p-3 text-sm"
              />
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="submit"
                  disabled={
                    action.isPending || !session.connected || !text.trim()
                  }
                >
                  {t("social.send")}
                </Button>
                <Button
                  variant="secondary"
                  disabled={action.isPending || !session.connected}
                  onClick={() =>
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
                >
                  {t("social.attach")}
                </Button>
                {selected?.kind === "direct" ? <SharePack key={selected.id} channel={selected.id}
                  disabled={action.isPending || !session.connected || !selected.peer?.canInvite || selected.peer?.isBlocked === true} /> : null}
                <span className="text-xs text-muted">
                  {t("social.fileLimit")}
                </span>
              </div>
            </form>
          </section>
        </div>
      ) : null}
      {tab === "friends" ? (
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
      ) : null}
      {tab === "privacy" && privacy.data ? (
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
      ) : null}
      {tab === "admin" && canModerate(self.role) ? (
        <SocialAdmin self={self} />
      ) : null}
      {profile ? (
        <Profile
          user={profile}
          close={() => setProfile(null)}
          direct={direct}
          run={run}
          invite={() => {
            if (room.data)
              run(async () => {
                const channel = unwrap(
                  await api.socialOpenDirect(profile.uuid),
                );
                unwrap(
                  await api.socialSend(channel.id, {
                    type: "roomInvite",
                    code: room.data!.code,
                    link: `mineded://join/${room.data!.code}`,
                  }),
                );
              });
          }}
          canInvite={!!room.data && profile.canInvite}
        />
      ) : null}
    </div>
  );
}

function MessageContent({
  message,
  save,
}: {
  message: SocialMessage;
  save: (
    attachment: Extract<SocialContent, { type: "attachment" }>["attachment"],
  ) => void;
}) {
  const { t } = useTranslation();
  const content = message.content;
  if (content.type === "text")
    return (
      <p className="mt-2 whitespace-pre-wrap break-words">{content.text}</p>
    );
  if (content.type === "attachment")
    return (
      <div className="mt-2">
        <p className="whitespace-pre-wrap break-words">{content.caption}</p>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => save(content.attachment)}
        >
          {t("social.saveFile", {
            name: content.attachment.fileName,
            size: (content.attachment.size / 1024).toFixed(0),
          })}
        </Button>
        <p className="text-xs text-muted">{content.attachment.mime}</p>
        {content.attachment.mime.startsWith("image/") ? (
          <AttachmentPreview id={content.attachment.id} />
        ) : null}
      </div>
    );
  if (content.type === "roomInvite")
    return (
      <div className="mt-2 rounded-lg bg-surface-2 p-2">
        <p>{t("social.roomInvite", { code: content.code })}</p>
        <Button
          size="sm"
          onClick={() => useNavStore.getState().openJoin(content.code)}
        >
          {t("social.join")}
        </Button>
      </div>
    );
  return (
    <div className="mt-2 rounded-lg bg-surface-2 p-2">
      <p className="font-medium">{content.name}</p>
      <p>
        {t("social.pack", {
          mc: content.mc,
          loader: content.loader,
          mods: content.mods,
        })}
      </p>
      <ReceivePack pack={content} offline={message.author.status === "offline"} />
    </div>
  );
}

function Profile({
  user: initialUser,
  close,
  direct,
  run,
  invite,
  canInvite,
}: {
  user: SocialUser;
  close: () => void;
  direct: (user: SocialUser) => void;
  run: (fn: () => Promise<unknown>) => void;
  invite: () => void;
  canInvite: boolean;
}) {
  const { t } = useTranslation();
  const { reduced, light } = useMotionPrefs();
  const refreshed = useQuery({
    queryKey: ["social", "profile", initialUser.uuid],
    queryFn: async () => unwrap(await api.socialProfile(initialUser.uuid)),
  });
  const user = refreshed.data ?? initialUser;
  const locallyBlocked = useSocialStore((s) => s.blocked.includes(user.uuid));
  const blocked = user.isBlocked === true || locallyBlocked;
  return (
    <Modal open onClose={close} title={t("social.profile")}>
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-semibold">{user.name}</h3>
          <Button size="sm" variant="ghost" onClick={close}>
            {t("common.close")}
          </Button>
        </div>
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
        <p>{t(`social.status.${user.status ?? "hidden"}`)}</p>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!canContact(user, blocked)}
            onClick={() => direct(user)}
          >
            {t("social.write")}
          </Button>
          <Button
            variant="secondary"
            disabled={!canInvite || blocked}
            onClick={invite}
          >
            {t("social.invite")}
          </Button>
          <Button
            variant="secondary"
            onClick={() =>
              run(async () => {
                if (user.isFriend)
                  unwrap(await api.socialRemoveFriend(user.uuid));
                else unwrap(await api.socialAddFriend(user.uuid));
              })
            }
          >
            {t(user.isFriend ? "social.removeFriend" : "social.addFriend")}
          </Button>
          <Button
            variant="ghost"
            onClick={() =>
              run(async () => {
                unwrap(await api.socialSetBlocked(user.uuid, !blocked));
                useSocialStore.getState().block(user.uuid, !blocked);
              })
            }
          >
            {t(blocked ? "social.unblock" : "social.block")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
