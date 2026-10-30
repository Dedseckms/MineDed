import type {
  AdminServerState,
  SocialChannel,
  SocialContent,
  SocialFriend,
  SocialMessage,
  SocialPrivacy,
  SocialReport,
  SocialRole,
  SocialUser,
  commands,
} from "../bindings";
import type { NetMockContext } from "./net";
import { canModerate, privacyOptions } from "../lib/social";
import { validateNick } from "../lib/validation";

type Commands = typeof commands;
const ok = <T>(data: T) => ({ status: "ok" as const, data });
const denied = () => ({
  status: "error" as const,
  error: {
    code: "permissionDenied" as const,
    message: "mock: operation denied",
    details: null,
  },
});
/** Launchers "on the wire" in the mock, any account kind. */
export const MOCK_ONLINE = 12;

export const SOCIAL_PEER: SocialUser = {
  uuid: "22222222222222222222222222222222",
  name: "Alex",
  skinUrl: null,
  status: "online",
  role: "user",
  isBlocked: false,
  isFriend: false,
  canMessage: true,
  canInvite: true,
};

/** Stateful social service. `role` is injectable for authorization tests only. */
export function createSocialMocks(
  ctx: Pick<NetMockContext, "bus" | "findAccount" | "runTask">,
  role: SocialRole = "user",
): Partial<Commands> {
  let self: SocialUser | null = null;
  let privacy: SocialPrivacy = {
    whoCanMessage: "everyone",
    whoCanInvite: "everyone",
    whoSeesStatus: "everyone",
    whoCanAddFriend: "everyone",
  };
  const blocked = new Set<string>();
  let friends: SocialFriend[] = [
    { user: { ...SOCIAL_PEER }, state: "incoming" },
  ];
  let messages: SocialMessage[] = [];
  let channels: SocialChannel[] = [
    { id: "global", kind: "global", peer: null, lastMessage: null, unread: 0 },
  ];
  let reports: SocialReport[] = [];
  let seq = 0;
  const server: AdminServerState = {
    diskUsedBytes: 42 * 1024 ** 3,
    diskTotalBytes: 100 * 1024 ** 3,
    attachmentsPaused: false,
    lastBackup: new Date().toISOString(),
    backupError: null,
    migrationProgress: null,
  };
  const peer = (): SocialUser => ({
    ...SOCIAL_PEER,
    isBlocked: blocked.has(SOCIAL_PEER.uuid),
    isFriend: friends.some((f) => f.state === "friends"),
    canMessage: !blocked.has(SOCIAL_PEER.uuid),
    canInvite: !blocked.has(SOCIAL_PEER.uuid),
  });
  const send = (channel: string, content: SocialContent) => {
    if (
      !self ||
      self.role === "guest" ||
      !channels.some((c) => c.id === channel) ||
      (channel !== "global" && blocked.has(SOCIAL_PEER.uuid))
    )
      return denied();
    const message: SocialMessage = {
      id: String(++seq),
      channel,
      author: { ...self },
      content,
      createdAt: new Date().toISOString(),
      editedAt: null,
      deleted: false,
    };
    messages.push(message);
    channels = channels.map((c) =>
      c.id === channel ? { ...c, lastMessage: message } : c,
    );
    ctx.bus.emit("socialEvent", { type: "message", message });
    return ok(message);
  };
  const remove = (id: string) => {
    const message = messages.find((m) => m.id === id);
    if (!message) return denied();
    message.deleted = true;
    message.content = { type: "text", text: "" };
    ctx.bus.emit("socialEvent", { type: "messageUpdated", message });
    return ok(null);
  };
  return {
    socialConnect: async (account) => {
      const value = ctx.findAccount(account);
      if (
        !value ||
        value.kind !== "yggdrasil" ||
        !value.valid ||
        value.server?.includes("ely.by")
      )
        return denied();
      self = {
        uuid: value.uuid.replaceAll("-", ""),
        name: value.name,
        skinUrl: value.skinUrl,
        status: "online",
        role,
        isBlocked: false,
        isFriend: false,
        canMessage: true,
        canInvite: true,
      };
      ctx.bus.emit("socialEvent", { type: "connection", connected: true });
      return ok({ ...self });
    },
    socialDisconnect: async () => {
      self = null;
      ctx.bus.emit("socialEvent", { type: "connection", connected: false });
      return ok(null);
    },
    socialGuestLogin: async (name) => {
      const nick = name.trim();
      if (validateNick(nick) !== null)
        return {
          status: "error" as const,
          error: {
            code: "invalidInput" as const,
            message: "mock: bad guest name",
            details: null,
          },
        };
      self = {
        uuid: crypto.randomUUID().replaceAll("-", ""),
        name: nick,
        skinUrl: null,
        status: "online",
        role: "guest",
        isBlocked: false,
        isFriend: false,
        canMessage: false,
        canInvite: true,
        guest: true,
      };
      channels = channels.filter((c) => c.kind === "global");
      friends = [];
      ctx.bus.emit("socialEvent", { type: "connection", connected: true });
      return ok({ ...self });
    },
    socialPresence: async () => ok({ online: MOCK_ONLINE }),
    socialSetPresence: async (status) => {
      if (!self) return denied();
      self.status = status;
      return ok(null);
    },
    socialChannels: async () =>
      self ? ok(structuredClone(channels)) : denied(),
    socialHistory: async (channel, before) => {
      if (!self) return denied();
      const all = messages.filter(
        (m) =>
          m.channel === channel && (!before || Number(m.id) < Number(before)),
      );
      const page = all.slice(-30);
      return ok({
        messages: structuredClone(page),
        before: all.length > 30 ? (page[0]?.id ?? null) : null,
      });
    },
    socialMarkRead: async (channel) => {
      if (!self) return denied();
      channels = channels.map((c) =>
        c.id === channel ? { ...c, unread: 0 } : c,
      );
      return ok(null);
    },
    socialSend: async (channel, content) => send(channel, content),
    socialDeleteMessage: async (id) =>
      self && messages.find((m) => m.id === id)?.author.uuid === self.uuid
        ? remove(id)
        : denied(),
    socialOpenDirect: async (uuid) => {
      if (
        !self ||
        self.role === "guest" ||
        uuid !== SOCIAL_PEER.uuid ||
        blocked.has(uuid)
      )
        return denied();
      const id = `dm:${[self.uuid, uuid].sort().join(":")}`;
      let channel = channels.find((c) => c.id === id);
      if (!channel) {
        channel = {
          id,
          kind: "direct",
          peer: peer(),
          lastMessage: null,
          unread: 0,
        };
        channels.push(channel);
      }
      return ok(structuredClone(channel));
    },
    socialSearchUsers: async (query) =>
      self
        ? ok(
            SOCIAL_PEER.name.toLowerCase().includes(query.toLowerCase())
              ? [peer()]
              : [],
          )
        : denied(),
    socialProfile: async (uuid) =>
      self ? ok(uuid === self.uuid ? { ...self } : peer()) : denied(),
    socialFriends: async () => (self ? ok(structuredClone(friends)) : denied()),
    socialAddFriend: async (uuid) => {
      if (!self || self.role === "guest" || blocked.has(uuid))
        return denied();
      const existing = friends.find((f) => f.user.uuid === uuid);
      const friend: SocialFriend = {
        user: peer(),
        state: existing?.state === "incoming" ? "friends" : "outgoing",
      };
      friends = [...friends.filter((f) => f.user.uuid !== uuid), friend];
      ctx.bus.emit("socialEvent", { type: "friend", friend });
      return ok(friend);
    },
    socialRemoveFriend: async (uuid) => {
      if (!self) return denied();
      friends = friends.filter((f) => f.user.uuid !== uuid);
      ctx.bus.emit("socialEvent", { type: "friendRemoved", uuid });
      return ok(null);
    },
    socialSetBlocked: async (uuid, value) => {
      if (!self) return denied();
      if (value) blocked.add(uuid);
      else blocked.delete(uuid);
      return ok(null);
    },
    socialPrivacy: async () => (self ? ok({ ...privacy }) : denied()),
    socialSetPrivacy: async (next) => {
      if (
        !self ||
        (Object.keys(next) as (keyof SocialPrivacy)[]).some(
          (key) => !privacyOptions(key).includes(next[key]),
        )
      )
        return denied();
      privacy = { ...next };
      return ok({ ...privacy });
    },
    socialReport: async (id, reason) => {
      const message = messages.find((m) => m.id === id);
      if (!self || !message) return denied();
      reports.push({
        id: String(++seq),
        message,
        reporter: self,
        reason,
        createdAt: new Date().toISOString(),
      });
      return ok(null);
    },
    socialReports: async () =>
      self && canModerate(self.role) ? ok(structuredClone(reports)) : denied(),
    socialResolveReport: async (id) => {
      if (!self || !canModerate(self.role)) return denied();
      reports = reports.filter((r) => r.id !== id);
      return ok(null);
    },
    socialModerateDelete: async (id) =>
      self &&
      canModerate(self.role) &&
      messages.find((m) => m.id === id)?.channel === "global"
        ? remove(id)
        : denied(),
    socialModerateUser: async (_uuid, action, seconds) =>
      !self ||
      !canModerate(self.role) ||
      (self.role !== "admin" &&
        (action === "ban" ||
          action === "unban" ||
          (action === "mute" && (seconds === null || seconds > 86400))))
        ? denied()
        : ok(null),
    socialSetRole: async () => (self?.role === "admin" ? ok(null) : denied()),
    socialServerState: async () =>
      self?.role === "admin" ? ok({ ...server }) : denied(),
    socialMigrateStorage: async () => {
      if (self?.role !== "admin") return denied();
      server.migrationProgress = 0;
      ctx.bus.emit("socialEvent", { type: "admin", state: { ...server } });
      return ok(null);
    },
    socialSendFile: async (channel, path, caption, ch) => {
      if (!self) return denied();
      return ok(
        ctx.runTask(
          "chatTransfer",
          "tasks.chatSendFile",
          ch,
          [{ stage: "chat.upload", total: 1024 }],
          () => {
            const result = send(channel, {
              type: "attachment",
              caption,
              attachment: {
                id: `attachment-${++seq}`,
                fileName: path.split(/[\\/]/).pop() ?? "file",
                mime: "application/octet-stream",
                size: 1024,
                url: "",
                thumbUrl: null,
                width: null,
                height: null,
              },
            });
            return result.status === "error" ? result.error : null;
          },
        ),
      );
    },
    socialSaveAttachment: async (_id, _dest, ch) =>
      self
        ? ok(
            ctx.runTask(
              "chatTransfer",
              "tasks.chatSaveFile",
              ch,
              [{ stage: "chat.download", total: 1024 }],
              () => null,
            ),
          )
        : denied(),
    socialAttachmentPreview: async () => denied(),
  };
}
