import type {
  AccountRequest,
  AdminAccount,
  AdminServerState,
  AuditEntry,
  PresenceClient,
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
import { canModerate, initials, privacyOptions, userHue } from "../lib/social";
import { ACCENTS, MAX_BIO, MAX_DISPLAY_NAME } from "../lib/socialProfile";

export const MOCK_ONLINE = 12;

/** Stand-in picture: the hub serves a webp, the mock an inline SVG. */
function mockAvatarDataUrl(user: SocialUser): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">` +
    `<rect width="64" height="64" fill="hsl(${userHue(user.uuid)} 58% 42%)"/>` +
    `<text x="32" y="43" text-anchor="middle" font-family="sans-serif" font-size="26" fill="#fff">` +
    `${initials(user.name)}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

type Commands = typeof commands;
const ok = <T>(data: T) => ({ status: "ok" as const, data });
const invalid = (message: string) => ({
  status: "error" as const,
  error: { code: "invalidInput" as const, message, details: null },
});
const denied = () => ({
  status: "error" as const,
  error: {
    code: "permissionDenied" as const,
    message: "mock: operation denied",
    details: null,
  },
});
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
  displayName: null,
  bio: null,
  accent: null,
  avatar: "mock-avatar-1",
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
  let seq = 0;
  // A couple of messages from another player, so the global chat has something
  // to hover (report, moderator delete) before anyone types.
  const before = Date.now() - 4 * 60 * 1000;
  let messages: SocialMessage[] = [
    {
      id: String(++seq),
      channel: "global",
      author: { ...SOCIAL_PEER },
      content: { type: "text", text: "Кто-нибудь хочет в комнату на ванилле?" },
      createdAt: new Date(before).toISOString(),
      editedAt: null,
      deleted: false,
    },
    {
      id: String(++seq),
      channel: "global",
      author: { ...SOCIAL_PEER },
      content: { type: "text", text: "Сборка уже стоит, заходите :)" },
      createdAt: new Date(before + 60_000).toISOString(),
      editedAt: null,
      deleted: false,
    },
  ];
  let channels: SocialChannel[] = [
    { id: "global", kind: "global", peer: null, lastMessage: messages[1] ?? null, unread: 2 },
  ];
  let reports: SocialReport[] = [];
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
  let audit: AuditEntry[] = [
    {
      id: "audit-1",
      at: new Date(Date.now() - 3_600_000).toISOString(),
      actor: "Admin",
      action: "mute",
      target: SOCIAL_PEER.name,
      note: "1 h",
    },
    {
      id: "audit-2",
      at: new Date(Date.now() - 7_200_000).toISOString(),
      actor: "Admin",
      action: "role",
      target: SOCIAL_PEER.name,
      note: "moderator",
    },
    {
      id: "audit-3",
      at: new Date(Date.now() - 86_400_000).toISOString(),
      actor: "Moderator",
      action: "delete",
      target: "global",
      note: null,
    },
  ];
  /** Journal entry for the next admin action; called only after the checks. */
  const log = (action: string, target: string | null, note: string | null = null) => {
    if (!self) return;
    audit = [
      {
        id: `audit-${++seq}`,
        at: new Date().toISOString(),
        actor: self.name,
        action,
        target,
        note,
      },
      ...audit,
    ];
  };
  const targetName = (uuid: string) =>
    uuid === SOCIAL_PEER.uuid ? SOCIAL_PEER.name : uuid;
  // Like the hub: a real account can open a direct chat with anyone it may message.
  const directable = (user: SocialUser): SocialUser => ({
    ...user,
    isBlocked: blocked.has(user.uuid),
    isFriend: friends.some((f) => f.state === "friends"),
    canMessage: !blocked.has(user.uuid),
    canInvite: !blocked.has(user.uuid) && (!self?.guest || user.guest === true),
  });
  const userById = (uuid: string): SocialUser | null => {
    if (self?.uuid === uuid) return self;
    if (SOCIAL_PEER.uuid === uuid) return peer();
    return friends.find((f) => f.user.uuid === uuid)?.user ?? null;
  };
  const clients = (): PresenceClient[] => {
    const since = new Date(Date.now() - 900_000).toISOString();
    return [
      { label: self?.name ?? "anonymous", kind: "yggdrasil", since },
      { label: "Guest", kind: "guest", since },
      { label: "anonymous", kind: "anonymous", since },
    ];
  };
  // Account requests and the full account list behind the admin panel.
  const minutesAgo = (n: number) => new Date(Date.now() - n * 60_000).toISOString();
  let requests: AccountRequest[] = [
    {
      id: "44444444444444444444444444444444",
      name: "NotchFan",
      note: "Играю гостём третий день, хочу свой ник.",
      createdAt: minutesAgo(40),
    },
    {
      id: "55555555555555555555555555555555",
      name: "Kefir",
      note: "",
      createdAt: minutesAgo(60 * 26),
    },
  ];
  const approved = new Set<string>();
  const accounts = (): AdminAccount[] => [
    {
      uuid: SOCIAL_PEER.uuid,
      name: SOCIAL_PEER.name,
      guest: false,
      displayName: null,
      createdAt: minutesAgo(60 * 24 * 12),
      invite: null,
      inviteUrl: null,
    },
    {
      uuid: "66666666666666666666666666666666",
      name: "Gremlin",
      guest: true,
      displayName: "Gremlin_",
      createdAt: minutesAgo(60 * 5),
      invite: null,
      inviteUrl: null,
    },
    ...requests
      .filter((r) => approved.has(r.id))
      .map((r) => ({
        uuid: r.id,
        name: r.name,
        guest: false,
        displayName: null,
        createdAt: r.createdAt,
        invite: null,
        inviteUrl: null,
      })),
  ];
  const send = (channel: string, content: SocialContent) => {
    if (
      !self ||
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
      if (!value || value.valid === false) return denied();
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
        displayName: null,
        bio: null,
        accent: null,
      };
      ctx.bus.emit("socialEvent", { type: "connection", connected: true });
      return ok({ ...self });
    },
    socialDisconnect: async () => {
      self = null;
      ctx.bus.emit("socialEvent", { type: "connection", connected: false });
      return ok(null);
    },
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
      if (!self || uuid === self.uuid) return denied();
      const user = userById(uuid);
      if (!user || blocked.has(uuid)) return denied();
      // A plain registered player may not DM another player who refuses messages.
      if (!self.guest && !user.guest && !user.canMessage) return denied();
      const id = `dm:${[self.uuid, user.uuid].sort().join(":")}`;
      let channel = channels.find((c) => c.id === id);
      if (!channel) {
        channel = {
          id,
          kind: "direct",
          peer: directable(user),
          lastMessage: null,
          unread: 0,
        };
        channels.push(channel);
      }
      return ok({ ...structuredClone(channel), peer: directable(user) });
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
      self
        ? ok(
            uuid === self.uuid
              ? { ...self, accountCreated: approved.has(self.uuid) }
              : peer(),
          )
        : denied(),
    socialFriends: async () => (self ? ok(structuredClone(friends)) : denied()),
    socialAddFriend: async (uuid) => {
      if (!self || blocked.has(uuid)) return denied();
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
      const report = reports.find((r) => r.id === id);
      reports = reports.filter((r) => r.id !== id);
      log("resolve", report ? report.message.author.name : null);
      return ok(null);
    },
    socialModerateDelete: async (id) => {
      const message = messages.find((m) => m.id === id);
      if (
        !self ||
        !canModerate(self.role) ||
        message?.channel !== "global"
      )
        return denied();
      log("delete", message.author.name);
      return remove(id);
    },
    socialModerateUser: async (uuid, action, seconds) => {
      if (
        !self ||
        !canModerate(self.role) ||
        (self.role !== "admin" &&
          (action === "ban" ||
            action === "unban" ||
            (action === "mute" && (seconds === null || seconds > 86400))))
      )
        return denied();
      log(action, targetName(uuid), seconds === null ? null : `${seconds} s`);
      return ok(null);
    },
    socialSetRole: async (uuid, role) => {
      if (self?.role !== "admin") return denied();
      log("role", targetName(uuid), role);
      return ok(null);
    },
    socialServerState: async () =>
      self?.role === "admin" ? ok({ ...server }) : denied(),
    socialMigrateStorage: async (target) => {
      if (self?.role !== "admin") return denied();
      log("migrate", target, null);
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
    socialGuestLogin: async (name) => {
      if (name.trim().length < 3) return invalid("mock: short guest name");
      self = {
        ...SOCIAL_PEER,
        uuid: "33333333333333333333333333333333",
        name: name.trim(),
        role: "guest",
        guest: true,
        isFriend: false,
        canMessage: false,
        canInvite: false,
      };
      ctx.bus.emit("socialEvent", { type: "connection", connected: true });
      return ok({ ...self });
    },
    socialProfileUpdate: async (patch) => {
      if (!self || self.role === "guest" || self.guest) return denied();
      const next: SocialUser = { ...self };
      if (patch.displayName != null) {
        const name = patch.displayName.trim();
        if (name.length > MAX_DISPLAY_NAME)
          return invalid("mock: display name is too long");
        next.displayName = name === "" ? null : name;
      }
      if (patch.bio != null) {
        if (patch.bio.length > MAX_BIO) return invalid("mock: bio is too long");
        next.bio = patch.bio === "" ? null : patch.bio;
      }
      if (patch.accent != null) {
        if (patch.accent !== "" && !ACCENTS.some((a) => a.id === patch.accent))
          return invalid("mock: unknown accent");
        next.accent = patch.accent === "" ? null : patch.accent;
      }
      self = next;
      return ok({ ...self });
    },
    socialAvatarSet: async (path, ch) => {
      if (!self || self.role === "guest" || self.guest) return denied();
      if (!/\.(png|jpe?g|webp)$/i.test(path)) return invalid("mock: not an image");
      return ok(
        ctx.runTask(
          "chatTransfer",
          "tasks.chatAvatar",
          ch,
          [{ stage: "chat.upload", total: 256 * 1024 }],
          () => {
            if (self) self.avatar = `mock-avatar-${++seq}`;
            return null;
          },
        ),
      );
    },
    socialAvatarClear: async () => {
      if (!self || self.role === "guest") return denied();
      self.avatar = null;
      return ok({ ...self });
    },
    socialAvatarPreview: async (uuid) => {
      if (!self) return denied();
      const user = userById(uuid);
      if (!user?.avatar) return invalid("mock: no avatar");
      return ok(mockAvatarDataUrl(user));
    },
    socialPresence: async () => ok({ online: 12 }),
    socialRequestAccount: async (note, ...rest) => {
      // 1.5.1 shape: (login, password, note). The extra args arrive untyped
      // until the bindings expose them (see lib/socialAdmin.ts).
      const args = [note, ...rest] as unknown[];
      const [login, password, text] =
        args.length >= 3
          ? [String(args[0]), String(args[1]), String(args[2])]
          : [null, null, String(note)];
      if (!self || (!self.guest && self.role !== "guest")) return denied();
      if (text.length > 300) return invalid("mock: note is too long");
      if (login !== null && (password ?? "").length < 6) return invalid("mock: short password");
      if (requests.some((r) => r.id === self!.uuid) || approved.has(self!.uuid))
        return {
          status: "error" as const,
          error: {
            code: "conflict" as const,
            message: "mock: account request already sent",
            details: null,
          },
        };
      requests = [
        ...requests,
        {
          id: self.uuid,
          name: login ?? self.name,
          note: text,
          createdAt: new Date().toISOString(),
        },
      ];
      self = { ...self, name: login ?? self.name };
      return ok(null);
    },
    socialAccountRequests: async () =>
      self?.role === "admin"
        ? ok(requests.map((r) => ({ ...r })))
        : denied(),
    socialIssueInvite: async (requestId) => {
      // 1.5.1: creates the account with the requested login, no invite link.
      if (self?.role !== "admin") return denied();
      const request = requests.find((r) => r.id === requestId);
      if (!request) return invalid("mock: unknown request");
      if (approved.has(requestId))
        return {
          status: "error" as const,
          error: {
            code: "conflict" as const,
            message: "mock: account already created",
            details: { reason: "usernameTaken" },
          },
        };
      if ([...approved].some((id) => requests.find((r) => r.id === id)?.name === request.name))
        return {
          status: "error" as const,
          error: {
            code: "conflict" as const,
            message: "mock: username is taken",
            details: { reason: "usernameTaken" },
          },
        };
      approved.add(requestId);
      return ok("");
    },
    socialAdminAccounts: async () =>
      self?.role === "admin" ? ok(accounts()) : denied(),
    socialAdminPresence: async () =>
      self?.role === "admin"
        ? ok({ online: clients().length, clients: clients() })
        : denied(),
    socialAdminAudit: async () =>
      self?.role === "admin" ? ok(structuredClone(audit)) : denied(),
  };
}
