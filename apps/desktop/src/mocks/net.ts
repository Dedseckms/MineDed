// Mock VPN (mihomo) and P2P rooms/chat. Stateful; emits the same events as
// the real backend through the mock bus. Owned by ui.
import type {
  AppError,
  AudioDevice,
  ChatMessage,
  Instance,
  InstanceId,
  InstanceSig,
  Peer,
  RoomState,
  TaskKind,
  VoiceSettings,
  VpnNode,
  VpnState,
  commands,
} from "../bindings";
import { CHAT_MAX, ROOM_LINK_PREFIX, parseJoinInput } from "../lib/rooms";
import { GAIN_MAX, GAIN_MIN, SELF_PEER_ID, withPeerVolume } from "../lib/voice";
import { parseSubscriptionUrls, validateSubscriptionPath } from "../lib/vpn";
import type { MockBus } from "./bus";

type Commands = typeof commands;
type Channel = Parameters<Commands["vpnRefresh"]>[0];

export interface NetMockContext {
  bus: MockBus;
  tickMs: number;
  wait: () => Promise<void>;
  findInstance: (id: InstanceId) => Instance | undefined;
  /** Name of the default account (sender of own chat messages). */
  selfName: () => string;
  runTask: (
    kind: TaskKind,
    titleKey: string,
    ch: Channel,
    stages: Array<{ stage: string; total: number | null }>,
    onSuccess: () => AppError | null,
  ) => string;
}

const err = (code: AppError["code"], message: string, details: Record<string, string> | null = null) => ({
  status: "error" as const,
  error: { code, message, details } satisfies AppError,
});
const ok = <T,>(data: T) => ({ status: "ok" as const, data });

/** Nodes a mock subscription "downloads". */
export const MOCK_NODES: ReadonlyArray<Pick<VpnNode, "name" | "protocol">> = [
  { name: "🇳🇱 Amsterdam", protocol: "vless" },
  { name: "🇩🇪 Frankfurt", protocol: "vless" },
  { name: "🇫🇮 Helsinki", protocol: "hysteria2" },
  { name: "🇵🇱 Warsaw", protocol: "trojan" },
  { name: "🇹🇷 Istanbul", protocol: "shadowsocks" },
  { name: "🇺🇸 New York", protocol: "vmess" },
  { name: "🇯🇵 Tokyo", protocol: "hysteria2" },
  { name: "🇰🇿 Almaty", protocol: "vless" },
  { name: "🇸🇬 Singapore", protocol: "trojan" },
];

/** Deterministic pseudo latency per node and round; every 7th is a timeout. */
export function mockLatency(name: string, round: number): number | null {
  let h = round * 31;
  for (const ch of name) h = (h * 33 + ch.charCodeAt(0)) >>> 0;
  if (h % 7 === 0) return null;
  return 30 + (h % 470);
}

export const MOCK_PEER_NAMES = ["AlexBuilder", "Creeper_Hunter", "nyan_cat"] as const;

const CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

function makeCode(seed: number): string {
  let s = seed * 7919 + 17;
  let out = "";
  for (let i = 0; i < 8; i += 1) {
    s = (s * 1103515245 + 12345) >>> 0;
    out += CODE_CHARS[s % CODE_CHARS.length];
    if (i === 3) out += "-";
  }
  return out;
}

function sigOf(inst: Instance): InstanceSig {
  return {
    mc: inst.mc,
    loader: inst.loader.kind,
    loaderVersion: inst.loader.version,
    modsHash: "0".repeat(64),
  };
}

export function createNetMocks(ctx: NetMockContext): Partial<Commands> {
  const { bus, tickMs, wait } = ctx;

  // ---- VPN ----
  let urls: string[] = [];
  let nodes: VpnNode[] = [];
  let pingRound = 0;
  let vpn: VpnState = { mode: "off", running: false, node: null, latencyMs: null, subscriptionUpdated: null, error: null };

  const loadNodes = () => {
    const prev = nodes.find((n) => n.selected)?.name;
    nodes = MOCK_NODES.map((n, i) => ({
      ...n,
      latencyMs: null,
      selected: prev ? n.name === prev : i === 0,
    }));
    const sel = nodes.find((n) => n.selected) ?? null;
    vpn = { ...vpn, subscriptionUpdated: new Date().toISOString(), node: vpn.running ? (sel?.name ?? null) : vpn.node, error: null };
  };
  const emitVpn = () => bus.emit("vpnState", { ...vpn });

  // ---- rooms ----
  let room: RoomState | null = null;
  let roomSeq = 0;
  let msgSeq = 0;
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const later = (ms: number, fn: () => void) => {
    const id = setTimeout(() => {
      timers.delete(id);
      fn();
    }, ms);
    timers.add(id);
  };
  const clearTimers = () => {
    for (const id of timers) clearTimeout(id);
    timers.clear();
  };
  const emitRoom = () => bus.emit("roomState", room ? structuredClone(room) : null);
  const peerMsg = (peer: Peer, text: string, viaRelay = false): ChatMessage => {
    msgSeq += 1;
    return { id: `msg-${msgSeq}`, fromPeer: peer.peerId, fromName: peer.name, text, sentAt: new Date().toISOString(), viaRelay };
  };
  const updatePeer = (roomId: string, peer: Peer, joined = true) => {
    if (!room || room.roomId !== roomId) return;
    room = {
      ...room,
      peers: joined
        ? [...room.peers.filter((p) => p.peerId !== peer.peerId), peer]
        : room.peers.filter((p) => p.peerId !== peer.peerId),
    };
    bus.emit("roomPeer", { peer: { ...peer }, joined });
  };

  // ---- voice ----
  let voiceSettings = defaultVoiceSettings();
  let selfMuted = false;
  let voiceTimer: ReturnType<typeof setInterval> | null = null;
  let micTimer: ReturnType<typeof setInterval> | null = null;
  const stopVoice = () => {
    if (voiceTimer) clearInterval(voiceTimer);
    voiceTimer = null;
  };
  /** Connected peers take turns speaking; own mic is "on" unless muted. */
  const startVoice = () => {
    stopVoice();
    let step = 0;
    voiceTimer = setInterval(() => {
      step += 1;
      const peers = room?.peers.filter((p) => p.path !== "connecting") ?? [];
      const turn = peers.length > 0 ? Math.floor(step / 3) % (peers.length + 1) : -1;
      peers.forEach((p, i) => {
        const on = i === turn;
        bus.emit("voiceSpeaking", { peerId: p.peerId, speaking: on, level: on ? mockMicLevel(step) : 0 });
      });
      const self = !selfMuted && turn === peers.length;
      bus.emit("voiceSpeaking", { peerId: SELF_PEER_ID, speaking: self, level: self ? mockMicLevel(step) : 0 });
    }, tickMs * 2);
  };

  /** A friend connects: connecting → direct/relay, then says hi. */
  const scheduleGuest = (roomId: string, idx: number, delay: number) => {
    const name = MOCK_PEER_NAMES[idx % MOCK_PEER_NAMES.length] ?? "Guest";
    const base: Peer = { peerId: `peer-${roomId}-${idx}`, name, uuid: null, path: "connecting", rttMs: null, speaking: false, muted: false };
    later(delay, () => updatePeer(roomId, base));
    later(delay + tickMs * 4, () => {
      const peer: Peer = { ...base, path: idx % 2 === 0 ? "direct" : "relay", rttMs: idx % 2 === 0 ? 24 + idx * 7 : 85 + idx * 11 };
      updatePeer(roomId, peer);
      if (room?.roomId === roomId) bus.emit("chatMessage", peerMsg(peer, idx === 0 ? "Привет! / Hi!" : "o/", idx % 2 === 1));
    });
  };

  return {
    // ---- VPN ----
    vpnState: async () => {
      await wait();
      return ok({ ...vpn });
    },

    vpnSetSubscription: async (list) => {
      await wait();
      const parsed = parseSubscriptionUrls(list.join("\n"));
      if (!parsed.ok) return err("invalidInput", "mock: bad subscription urls");
      urls = parsed.urls;
      return ok(null);
    },

    vpnImportSubscriptionFile: async (path) => {
      await wait();
      if (validateSubscriptionPath(path) !== null) return err("invalidInput", "mock: bad path");
      if (/missing/i.test(path)) return err("notFound", "mock: file not found", { path });
      if (/broken/i.test(path)) return err("invalidInput", "mock: unrecognised subscription format", { path });
      loadNodes();
      emitVpn();
      return ok(null);
    },

    vpnRefresh: async (ch) => {
      await wait();
      if (urls.length === 0) return err("invalidInput", "mock: no subscription configured");
      const id = ctx.runTask("vpnUpdate", "tasks.vpnUpdate", ch, [{ stage: "stages.download", total: 240_000 }], () => {
        if (urls.some((u) => /blocked/i.test(u))) {
          const e: AppError = { code: "blocked", message: "mock: subscription host blocked", details: { host: "sub.blocked.invalid" } };
          bus.emit("netBlocked", { host: "sub.blocked.invalid", suggestionKey: "net.blockedHint" });
          return e;
        }
        loadNodes();
        emitVpn();
        return null;
      });
      return ok(id);
    },

    vpnSetMode: async (mode) => {
      await wait();
      if (mode !== "off" && nodes.length === 0) return err("invalidInput", "mock: no nodes, import a subscription first");
      const sel = nodes.find((n) => n.selected) ?? null;
      vpn =
        mode === "off"
          ? { ...vpn, mode, running: false, node: null, latencyMs: null, error: null }
          : { ...vpn, mode, running: true, node: sel?.name ?? null, latencyMs: sel?.latencyMs ?? null, error: null };
      emitVpn();
      return ok({ ...vpn });
    },

    vpnNodes: async () => {
      await wait();
      return ok(nodes.map((n) => ({ ...n })));
    },

    vpnSelect: async (name) => {
      await wait();
      const node = nodes.find((n) => n.name === name);
      if (!node) return err("notFound", `mock: node ${name} not found`);
      nodes = nodes.map((n) => ({ ...n, selected: n.name === name }));
      if (vpn.running) {
        vpn = { ...vpn, node: name, latencyMs: node.latencyMs };
        emitVpn();
      }
      return ok(null);
    },

    vpnTestLatency: async () => {
      await wait();
      pingRound += 1;
      nodes = nodes.map((n) => ({ ...n, latencyMs: mockLatency(n.name, pingRound) }));
      const sel = nodes.find((n) => n.selected);
      if (vpn.running && sel) {
        vpn = { ...vpn, latencyMs: sel.latencyMs };
        emitVpn();
      }
      return ok(nodes.map((n) => ({ ...n })));
    },

    // ---- rooms ----
    roomState: async () => {
      await wait();
      return ok(room ? structuredClone(room) : null);
    },

    roomCreate: async (instance, authMode, password) => {
      await wait();
      const inst = ctx.findInstance(instance);
      if (!inst) return err("notFound", `mock: instance ${instance} not found`);
      if (room) return err("conflict", "mock: already in a room");
      if (password !== null && password.length < 4) return err("invalidInput", "mock: password too short");
      roomSeq += 1;
      const code = makeCode(roomSeq);
      room = {
        roomId: `room-${roomSeq}`,
        code,
        role: "host",
        authMode,
        kind: "game",
        sig: sigOf(inst),
        peers: [],
        expiresAt: new Date(Date.now() + 6 * 3600_000).toISOString(),
        link: `${ROOM_LINK_PREFIX}${code}`,
        passwordProtected: password !== null,
        localAddr: null,
      };
      emitRoom();
      scheduleGuest(room.roomId, 0, tickMs * 10);
      scheduleGuest(room.roomId, 1, tickMs * 30);
      return ok(structuredClone(room));
    },

    roomCreateVoice: async (password) => {
      await wait();
      if (room) return err("conflict", "mock: already in a room");
      if (password !== null && password.length < 4) return err("invalidInput", "mock: password too short");
      roomSeq += 1;
      const code = makeCode(roomSeq);
      room = {
        roomId: `room-${roomSeq}`,
        code,
        role: "host",
        authMode: "open",
        kind: "voice",
        sig: null,
        peers: [],
        expiresAt: new Date(Date.now() + 6 * 3600_000).toISOString(),
        link: `${ROOM_LINK_PREFIX}${code}`,
        passwordProtected: password !== null,
        localAddr: null,
      };
      emitRoom();
      scheduleGuest(room.roomId, 0, tickMs * 10);
      scheduleGuest(room.roomId, 2, tickMs * 30);
      return ok(structuredClone(room));
    },

    // Codes starting with "VOX" are voice rooms (instance ignored); joining a
    // game room needs an instance.
    roomJoin: async (input, instance, password) => {
      await wait();
      const inst = instance ? ctx.findInstance(instance) : undefined;
      if (instance !== null && !inst) return err("notFound", `mock: instance ${instance} not found`);
      if (room) return err("conflict", "mock: already in a room");
      const parsed = parseJoinInput(input);
      if (!parsed.ok) return err("invalidInput", "mock: bad room code");
      const code = parsed.code;
      if (code.startsWith("0000")) return err("notFound", `mock: room ${code} not found`, { code });
      // Codes starting with "LOCK" are password-protected; the password is "secret".
      const locked = code.startsWith("LOCK");
      if (locked && password !== "secret") return err("permissionDenied", "mock: wrong room password");
      const voice = code.startsWith("VOX");
      if (!voice && !inst) return err("invalidInput", "mock: this is a game room, pick an instance", { kind: "game" });
      if (code.startsWith("SIG")) return err("conflict", "mock: instance does not match the host", { expected: "1.21.1 fabric" });
      roomSeq += 1;
      const roomId = `room-${roomSeq}`;
      const hostPeer: Peer = { peerId: `peer-${roomId}-host`, name: "Steve_Host", uuid: null, path: "relay", rttMs: 92, speaking: false, muted: false };
      room = {
        roomId,
        code,
        role: "guest",
        authMode: "open",
        kind: voice ? "voice" : "game",
        sig: !voice && inst ? sigOf(inst) : null,
        peers: [hostPeer],
        expiresAt: new Date(Date.now() + 6 * 3600_000).toISOString(),
        link: `${ROOM_LINK_PREFIX}${code}`,
        passwordProtected: locked,
        localAddr: voice ? null : `127.0.0.1:${25565 + roomSeq}`,
      };
      emitRoom();
      // Hole punching succeeds a bit later.
      later(tickMs * 8, () => updatePeer(roomId, { ...hostPeer, path: "direct", rttMs: 31 }));
      return ok(structuredClone(room));
    },

    roomLeave: async () => {
      await wait();
      clearTimers();
      stopVoice();
      if (room) {
        room = null;
        emitRoom();
      }
      return ok(null);
    },

    roomKick: async (peerId) => {
      await wait();
      if (!room) return err("notFound", "mock: not in a room");
      if (room.role !== "host") return err("permissionDenied", "mock: only the host can kick");
      const peer = room.peers.find((p) => p.peerId === peerId);
      if (!peer) return err("notFound", `mock: peer ${peerId} not found`);
      updatePeer(room.roomId, peer, false);
      return ok(null);
    },

    chatSend: async (text) => {
      await wait();
      if (!room) return err("notFound", "mock: not in a room");
      const trimmed = text.trim();
      if (trimmed.length === 0 || trimmed.length > CHAT_MAX) return err("invalidInput", "mock: bad chat message");
      msgSeq += 1;
      const msg: ChatMessage = {
        id: `msg-${msgSeq}`,
        fromPeer: "peer-self",
        fromName: ctx.selfName(),
        text: trimmed,
        sentAt: new Date().toISOString(),
        viaRelay: false,
      };
      // Like the real backend may do: own messages are echoed as an event too.
      bus.emit("chatMessage", { ...msg });
      const roomId = room.roomId;
      const friend = room.peers.find((p) => p.path !== "connecting");
      if (friend && /\?$/.test(trimmed)) later(tickMs * 6, () => room?.roomId === roomId && bus.emit("chatMessage", peerMsg(friend, "+")));
      return ok(msg);
    },

    // ---- voice ----
    voiceDevices: async () => {
      await wait();
      return ok(MOCK_AUDIO_DEVICES.map((d) => ({ ...d })));
    },

    voiceSettings: async () => {
      await wait();
      return ok(structuredClone(voiceSettings));
    },

    voiceSetSettings: async (next) => {
      await wait();
      const badGain = (v: number | null) => v !== null && (v < GAIN_MIN || v > GAIN_MAX);
      if (badGain(next.inputGain) || badGain(next.outputVolume)) return err("invalidInput", "mock: gain out of range");
      if (next.pttKey !== null && next.pttKey.trim() === "") return err("invalidInput", "mock: empty ptt key");
      const known = (id: string | null) => id === null || MOCK_AUDIO_DEVICES.some((d) => d.id === id);
      if (!known(next.inputDevice) || !known(next.outputDevice)) return err("notFound", "mock: audio device not found");
      voiceSettings = structuredClone(next);
      return ok(null);
    },

    voiceStart: async () => {
      await wait();
      if (!room) return err("notFound", "mock: not in a room");
      startVoice();
      return ok(null);
    },

    voiceStop: async () => {
      await wait();
      stopVoice();
      return ok(null);
    },

    voiceSetPeerMuted: async (peerId) => {
      await wait();
      if (!room?.peers.some((p) => p.peerId === peerId)) return err("notFound", `mock: peer ${peerId} not found`);
      return ok(null);
    },

    voiceSetPeerVolume: async (peerId, volume) => {
      await wait();
      const peer = room?.peers.find((p) => p.peerId === peerId);
      if (!peer) return err("notFound", `mock: peer ${peerId} not found`);
      if (volume !== null && (volume < GAIN_MIN || volume > GAIN_MAX)) return err("invalidInput", "mock: volume out of range");
      voiceSettings = withPeerVolume(voiceSettings, peer, volume ?? 1);
      return ok(null);
    },

    voiceSetSelfMuted: async (muted) => {
      await wait();
      selfMuted = muted;
      return ok(null);
    },

    voiceMicTest: async (enabled) => {
      await wait();
      if (micTimer) clearInterval(micTimer);
      micTimer = null;
      if (enabled) {
        let i = 0;
        micTimer = setInterval(() => {
          i += 1;
          const level = mockMicLevel(i);
          bus.emit("voiceSpeaking", { peerId: SELF_PEER_ID, speaking: level > 0.3, level });
        }, tickMs);
      } else {
        bus.emit("voiceSpeaking", { peerId: SELF_PEER_ID, speaking: false, level: 0 });
      }
      return ok(null);
    },
  };
}

/** Deterministic "voice" level 0..1 for the mic test meter. */
export function mockMicLevel(step: number): number {
  const v = Math.abs(Math.sin(step * 0.7)) * (step % 5 === 0 ? 0.2 : 0.9);
  return Math.round(v * 100) / 100;
}

export const MOCK_AUDIO_DEVICES: readonly AudioDevice[] = [
  { id: "in-default", name: "Microphone (Realtek Audio)", direction: "input", isDefault: true },
  { id: "in-usb", name: "USB Headset Mic", direction: "input", isDefault: false },
  { id: "out-default", name: "Speakers (Realtek Audio)", direction: "output", isDefault: true },
  { id: "out-usb", name: "USB Headset", direction: "output", isDefault: false },
];

export const defaultVoiceSettings = (): VoiceSettings => ({
  mode: "pushToTalk",
  inputDevice: null,
  outputDevice: null,
  pttKey: "KeyV",
  inputGain: 1,
  outputVolume: 1,
  noiseSuppression: true,
  peerVolumes: {},
});
