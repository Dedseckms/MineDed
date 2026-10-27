import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ChatMessage, RoomPeerEvent, RoomStateEvent, TaskEvent, VoiceSpeakingEvent, VoiceStatus, VpnState } from "../bindings";
import { unwrap } from "../api/client";
import { createMockBus } from "./bus";
import { createMockCommands } from "./index";
import { MOCK_NODES, mockLatency, mockMicLevel } from "./net";

const channel = (events: TaskEvent[]) =>
  ({ onmessage: (e: TaskEvent) => events.push(e) }) as unknown as Parameters<
    ReturnType<typeof createMockCommands>["vpnRefresh"]
  >[0];

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("mock vpn", () => {
  it("subscription → refresh task → nodes → mode → select → ping", async () => {
    const bus = createMockBus();
    const states: VpnState[] = [];
    bus.on("vpnState", (s) => states.push(s));
    const api = createMockCommands({}, { bus, tickMs: 10 });

    expect(unwrap(await api.vpnState())).toMatchObject({ mode: "off", running: false, subscriptionUpdated: null });
    expect((await api.vpnRefresh(channel([]))).status).toBe("error");
    expect((await api.vpnSetMode("launcherOnly")).status).toBe("error");
    expect((await api.vpnSetSubscription(["ftp://nope"])).status).toBe("error");

    unwrap(await api.vpnSetSubscription(["https://sub.example/abc"]));
    const events: TaskEvent[] = [];
    const id = unwrap(await api.vpnRefresh(channel(events)));
    await vi.advanceTimersByTimeAsync(500);
    expect(events.at(-1)).toEqual({ type: "finished", id, error: null });
    const nodes = unwrap(await api.vpnNodes());
    expect(nodes).toHaveLength(MOCK_NODES.length);
    expect(nodes.filter((n) => n.selected)).toHaveLength(1);

    const off = await api.vpnTestLatency();
    expect(off.status === "error" && off.error.code).toBe("invalidInput");
    const on = unwrap(await api.vpnSetMode("launcherAndGame"));
    expect(on).toMatchObject({ running: true, node: nodes[0]?.name });
    const target = nodes[2]?.name ?? "";
    unwrap(await api.vpnSelect(target));
    expect(states.at(-1)?.node).toBe(target);
    expect((await api.vpnSelect("nope")).status).toBe("error");

    const pinged = unwrap(await api.vpnTestLatency());
    expect(pinged.map((n) => n.latencyMs)).toEqual(MOCK_NODES.map((n) => mockLatency(n.name, 1)));
    expect(unwrap(await api.vpnSetMode("off"))).toMatchObject({ running: false, node: null });
  });

  it("file import validates the path; a blocked subscription host emits net://blocked", async () => {
    const bus = createMockBus();
    const blocked: string[] = [];
    bus.on("netBlocked", (p) => blocked.push(p.host));
    const api = createMockCommands({}, { bus, tickMs: 10 });
    expect((await api.vpnImportSubscriptionFile("relative.yaml")).status).toBe("error");
    const missing = await api.vpnImportSubscriptionFile("C:\\missing.yaml");
    expect(missing.status === "error" && missing.error.code).toBe("notFound");
    unwrap(await api.vpnImportSubscriptionFile("C:\\subs\\my.yaml"));
    expect(unwrap(await api.vpnNodes()).length).toBeGreaterThan(0);

    unwrap(await api.vpnSetSubscription(["https://blocked.example/sub"]));
    const events: TaskEvent[] = [];
    unwrap(await api.vpnRefresh(channel(events)));
    await vi.advanceTimersByTimeAsync(500);
    const last = events.at(-1);
    expect(last?.type === "finished" && last.error?.code).toBe("blocked");
    expect(blocked).toEqual(["sub.blocked.invalid"]);
  });

  it("keeps both subscriptions and switches the active source", async () => {
    const bus = createMockBus();
    const api = createMockCommands({}, { bus, tickMs: 10 });
    expect(unwrap(await api.vpnState())).toMatchObject({ source: null, hasOwn: false, hasMineded: false });
    expect((await api.vpnSetSource("own")).status).toBe("error");
    unwrap(await api.vpnImportSubscriptionFile("C:\\subs\\my.yaml"));
    expect(unwrap(await api.vpnState())).toMatchObject({ source: "own", hasOwn: true });
    expect(unwrap(await api.vpnNodes())).toHaveLength(MOCK_NODES.length);
    expect((await api.vpnSetSource("mineded")).status).toBe("error");
  });

  it("latency is deterministic and sometimes times out", () => {
    const all = MOCK_NODES.flatMap((n) => [1, 2, 3].map((r) => mockLatency(n.name, r)));
    expect(all.some((v) => v === null)).toBe(true);
    expect(all.filter((v) => v !== null).every((v) => v !== null && v >= 30 && v < 500)).toBe(true);
    expect(mockLatency("x", 4)).toBe(mockLatency("x", 4));
  });
});

describe("mock rooms", () => {
  it("host: create → friends join (connecting → direct/relay) → chat → kick → leave", async () => {
    const bus = createMockBus();
    const peers: RoomPeerEvent[] = [];
    const states: RoomStateEvent[] = [];
    const chat: ChatMessage[] = [];
    bus.on("roomPeer", (e) => peers.push(e));
    bus.on("roomState", (s) => states.push(s));
    bus.on("chatMessage", (m) => chat.push(m));
    const api = createMockCommands({}, { bus, tickMs: 10 });

    expect((await api.roomCreate("nope", "open", null)).status).toBe("error");
    expect((await api.roomCreate("inst-fabric", "open", "abc")).status).toBe("error");
    const room = unwrap(await api.roomCreate("inst-fabric", "yggdrasil", "secret"));
    expect(room).toMatchObject({ role: "host", authMode: "yggdrasil", passwordProtected: true, localAddr: null, peers: [] });
    expect(room.code).toMatch(/^[A-Z0-9]{4}-[A-Z0-9]{4}$/);
    expect(room.link).toBe(`mineded://join/${room.code}`);
    expect(room.sig).toMatchObject({ mc: "1.21.1", loader: "fabric" });
    expect((await api.roomCreate("inst-fabric", "open", null)).status).toBe("error");

    await vi.advanceTimersByTimeAsync(1000);
    const current = unwrap(await api.roomState());
    expect(current?.peers.map((p) => p.path)).toEqual(["direct", "relay"]);
    expect(peers[0]?.peer.path).toBe("connecting");
    expect(chat.length).toBeGreaterThan(0);

    const mine = unwrap(await api.chatSend("  hello?  "));
    expect(mine).toMatchObject({ text: "hello?", fromName: "Steve_42" });
    expect(chat.some((m) => m.id === mine.id)).toBe(true);
    expect((await api.chatSend("   ")).status).toBe("error");

    const kicked = current?.peers[0]?.peerId ?? "";
    unwrap(await api.roomKick(kicked));
    expect(peers.at(-1)).toMatchObject({ joined: false, peer: { peerId: kicked } });
    expect(unwrap(await api.roomState())?.peers).toHaveLength(1);

    unwrap(await api.roomLeave());
    expect(states.at(-1)).toBeNull();
    expect(unwrap(await api.roomState())).toBeNull();
    expect((await api.chatSend("hi")).status).toBe("error");
  });

  it("guest: join by link, password and sig checks, localAddr, no kick rights", async () => {
    const bus = createMockBus();
    const api = createMockCommands({}, { bus, tickMs: 10 });
    const err = async (p: Promise<{ status: string; error?: { code: string } }>) => {
      const r = await p;
      return r.status === "error" ? r.error?.code : "ok";
    };
    expect(await err(api.roomJoin("bad code!", "inst-fabric", null))).toBe("invalidInput");
    expect(await err(api.roomJoin("0000-0000", "inst-fabric", null))).toBe("notFound");
    expect(await err(api.roomJoin("LOCK-1234", "inst-fabric", null))).toBe("permissionDenied");
    expect(await err(api.roomJoin("SIGX-1234", "inst-fabric", null))).toBe("conflict");

    const room = unwrap(await api.roomJoin("mineup://join/lock-1234", "inst-fabric", "secret"));
    expect(room).toMatchObject({ role: "guest", code: "LOCK-1234", passwordProtected: true });
    expect(room.localAddr).toMatch(/^127\.0\.0\.1:\d+$/);
    expect(room.peers[0]?.path).toBe("relay");
    await vi.advanceTimersByTimeAsync(200);
    expect(unwrap(await api.roomState())?.peers[0]).toMatchObject({ path: "direct", rttMs: 31 });
    expect(await err(api.roomKick(room.peers[0]?.peerId ?? ""))).toBe("permissionDenied");
    unwrap(await api.roomLeave());
  });

  it("game room without instance: attach a matching one or download the host pack", async () => {
    const bus = createMockBus();
    const api = createMockCommands({}, { bus, tickMs: 10 });
    const code = async (p: Promise<{ status: string; error?: { code: string } }>) => {
      const r = await p;
      return r.status === "error" ? r.error?.code : "ok";
    };
    const game = unwrap(await api.roomJoin("ABCD-1234", null, null));
    expect(game.sig).toMatchObject({ mc: "1.21.1", loader: "fabric" });
    expect(await code(api.roomAttachInstance("inst-big"))).toBe("conflict");
    expect(await code(api.roomAttachInstance("inst-fabric"))).toBe("ok");
    unwrap(await api.roomLeave());

    unwrap(await api.roomJoin("PACK-1234", null, null));
    expect(await code(api.roomAttachInstance("inst-big"))).toBe("conflict");
    const events: TaskEvent[] = [];
    unwrap(await api.roomDownloadHostPack(channel(events)));
    await vi.advanceTimersByTimeAsync(500);
    expect(events.at(-1)).toMatchObject({ type: "finished", error: null });
    const created = unwrap(await api.instanceList()).find((i) => i.id.startsWith("inst-hostpack-"));
    expect(created).toMatchObject({ mc: "1.20.1", loader: { kind: "forge" } });
    expect(await code(api.roomAttachInstance(created?.id ?? ""))).toBe("ok");
    unwrap(await api.roomLeave());

    unwrap(await api.roomJoin("OLDH-1234", null, null));
    const old: TaskEvent[] = [];
    unwrap(await api.roomDownloadHostPack(channel(old)));
    await vi.advanceTimersByTimeAsync(500);
    const last = old.at(-1);
    expect(last?.type === "finished" && last.error?.details?.reason).toBe("hostTooOld");
  });

  it("voice room: create without instance; join VOX codes with instance=null", async () => {
    const bus = createMockBus();
    const api = createMockCommands({}, { bus, tickMs: 10 });
    expect((await api.roomCreateVoice("abc")).status).toBe("error");
    const room = unwrap(await api.roomCreateVoice(null));
    expect(room).toMatchObject({ kind: "voice", sig: null, localAddr: null, role: "host" });
    unwrap(await api.roomLeave());

    const joined = unwrap(await api.roomJoin("VOXA-1234", null, null));
    expect(joined).toMatchObject({ kind: "voice", sig: null, localAddr: null, role: "guest" });
    unwrap(await api.roomLeave());
    const withInst = unwrap(await api.roomJoin("VOXB-1234", "inst-fabric", null));
    expect(withInst.kind).toBe("voice");
    unwrap(await api.roomLeave());
  });

  it("leaving cancels pending simulated joins", async () => {
    const bus = createMockBus();
    const peers: RoomPeerEvent[] = [];
    bus.on("roomPeer", (e) => peers.push(e));
    const api = createMockCommands({}, { bus, tickMs: 10 });
    unwrap(await api.roomCreate("inst-fabric", "open", null));
    unwrap(await api.roomLeave());
    await vi.advanceTimersByTimeAsync(1000);
    expect(peers).toEqual([]);
  });
});

describe("mock voice", () => {
  it("settings validation, devices, peer volume persistence", async () => {
    const api = createMockCommands({}, { bus: createMockBus(), tickMs: 10 });
    const devices = unwrap(await api.voiceDevices());
    expect(devices.some((d) => d.direction === "input")).toBe(true);
    const s = unwrap(await api.voiceSettings());
    expect(s).toMatchObject({ mode: "pushToTalk", pttKey: "KeyV" });
    expect((await api.voiceSetSettings({ ...s, inputGain: 3 })).status).toBe("error");
    expect((await api.voiceSetSettings({ ...s, inputDevice: "nope" })).status).toBe("error");
    unwrap(await api.voiceSetSettings({ ...s, mode: "vad", inputDevice: "in-usb" }));
    expect(unwrap(await api.voiceSettings())).toMatchObject({ mode: "vad", inputDevice: "in-usb" });

    expect((await api.voiceStart()).status).toBe("error");
    const room = unwrap(await api.roomJoin("VOXA-0001", null, null));
    const host = room.peers[0];
    unwrap(await api.voiceSetPeerVolume(host?.peerId ?? "", 1.5));
    expect(unwrap(await api.voiceSettings()).peerVolumes).toEqual({ [host?.peerId ?? ""]: 1.5 });
    expect((await api.voiceSetPeerVolume(host?.peerId ?? "", 5)).status).toBe("error");
    expect((await api.voiceSetPeerMuted("ghost", true)).status).toBe("error");
    unwrap(await api.voiceSetPeerMuted(host?.peerId ?? "", true));
    unwrap(await api.roomLeave());
  });

  it("voice start emits speaking events; mic test emits self levels", async () => {
    const bus = createMockBus();
    const events: VoiceSpeakingEvent[] = [];
    const statuses: VoiceStatus[] = [];
    bus.on("voiceSpeaking", (e) => events.push(e));
    bus.on("voiceStatus", (s) => statuses.push(s));
    const api = createMockCommands({}, { bus, tickMs: 10 });
    expect(unwrap(await api.voiceStatus())).toMatchObject({ active: false, selfMuted: false, micTest: false });
    unwrap(await api.roomJoin("VOXA-0002", null, null));
    unwrap(await api.voiceStart());
    expect(statuses.at(-1)).toMatchObject({ active: true, mode: "pushToTalk" });
    unwrap(await api.voiceSetSelfMuted(true));
    expect(unwrap(await api.voiceStatus())).toMatchObject({ active: true, selfMuted: true });
    unwrap(await api.voiceSetSelfMuted(false));
    await vi.advanceTimersByTimeAsync(300);
    expect(events.some((e) => e.peerId !== "self" && e.speaking)).toBe(true);
    unwrap(await api.voiceStop());
    expect(statuses.at(-1)?.active).toBe(false);
    unwrap(await api.roomLeave());

    events.length = 0;
    unwrap(await api.voiceMicTest(true));
    await vi.advanceTimersByTimeAsync(100);
    expect(events.length).toBeGreaterThan(3);
    expect(events.every((e) => e.peerId === "self")).toBe(true);
    expect(statuses.at(-1)?.micTest).toBe(true);
    unwrap(await api.voiceMicTest(false));
    expect(statuses.at(-1)?.micTest).toBe(false);
    const n = events.length;
    await vi.advanceTimersByTimeAsync(100);
    expect(events.length).toBe(n);
    expect(events.at(-1)).toMatchObject({ speaking: false, level: 0 });
    expect(mockMicLevel(3)).toBe(mockMicLevel(3));
  });
});
