// Showcase mode for screenshots (mock mode only): `?demo=1` seeds a lived-in
// state — appearance, applied skin, a running VPN, a game room with friends,
// voice and chat. Optional params: `lang=ru|en`, `theme=dark|light`, `bg=<id>`.
// Owned by ui.
import type { ChatMessage, Settings, commands } from "../bindings";
import { createTaskChannel } from "../api/channel";
import { resolveBackground } from "../lib/backdrop";
import { useNavStore } from "../stores/navStore";
import { useRoomStore } from "../stores/roomStore";
import type { MockBus } from "./bus";

type Commands = typeof commands;

export interface DemoParams {
  lang: "ru" | "en";
  theme: "dark" | "light";
  background: string;
}

export function parseDemoParams(search: string): DemoParams | null {
  const q = new URLSearchParams(search);
  if (!q.has("demo")) return null;
  return {
    lang: q.get("lang") === "en" ? "en" : "ru",
    theme: q.get("theme") === "light" ? "light" : "dark",
    background: resolveBackground(q.get("bg")) ?? "forest",
  };
}

/** Command overrides: a release version and no pending update. */
export function demoOverrides(version: string): Partial<Commands> {
  return {
    appVersion: async () => version,
    updateCheck: async () => ({ status: "ok", data: null }),
  };
}

export function applyDemoSettings(s: Settings, p: DemoParams): Settings {
  return { ...s, language: p.lang, theme: p.theme, background: p.background, translucent: false };
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

const CHAT: Record<DemoParams["lang"], Array<[peer: 0 | 1 | "self", text: string]>> = {
  ru: [
    ["self", "Привет! Мир уже открыт, заходите"],
    [1, "Захожу, только моды докачаю"],
    [0, "Кто со мной в шахту за алмазами?"],
    ["self", "Я! Возьму кирки и факелы"],
  ],
  en: [
    ["self", "Hi! The world is up, join in"],
    [1, "Joining, just finishing the mod download"],
    [0, "Who's coming to the mine for diamonds?"],
    ["self", "Me! I'll bring pickaxes and torches"],
  ],
};

/** Seeds the showcase state; resolves before the chat history arrives (call before the first render). */
export async function runDemo(api: Commands, bus: MockBus, p: DemoParams): Promise<void> {
  const settings = await api.settingsGet();
  if (settings.status === "ok") await api.settingsSet(applyDemoSettings(settings.data, p));
  await api.skinApply("acc-steve", "skin-explorer");
  await api.capeSet("acc-steve", "cape-founder");

  await api.vpnSetSubscription(["https://sub.mineded.dedseckms.ru/s/7f3c9a"]);
  await api.vpnImportSubscriptionFile("C:\\Users\\Player\\Downloads\\mineup.yaml");
  await api.vpnSelect("🇫🇮 Helsinki");
  await api.vpnSetMode("launcherAndGame");
  await api.vpnTestLatency();

  // Borderless fullscreen keeps the voice panel compact (no "exclusive fullscreen" warning steps).
  const plan = await api.contentResolve("inst-fabric", [{ source: "modrinth", projectId: "UIexcMP1", versionId: null }]);
  if (plan.status === "ok") {
    // Wait for the task so the first content query already lists the mod.
    await new Promise<void>((resolve) => {
      const ch = createTaskChannel((ev) => ev.type === "finished" && resolve());
      void api.contentInstall("inst-fabric", plan.data, ch).then((r) => r.status === "error" && resolve());
    });
  }

  const room = await api.roomCreate("inst-fabric", "yggdrasil", null);
  if (room.status !== "ok") return;
  await api.voiceStart();
  // The room's instance is the one being played; it starts right after the first render.
  useNavStore.getState().selectPlayInstance("inst-fabric");
  await api.instanceLaunch(
    "inst-fabric",
    { account: null, joinServer: null, room: null, offlineFast: false },
    createTaskChannel(() => undefined),
  );
  void seedChat(bus, room.data.roomId, p);
}

/** Guests join after ~1.5 s and ~4.5 s (see `net.ts`); the chat history follows. */
async function seedChat(bus: MockBus, roomId: string, p: DemoParams): Promise<void> {
  await sleep(5500);
  const peerId = (i: 0 | 1) => `peer-${roomId}-${i}`;
  const names = ["AlexBuilder", "Creeper_Hunter"] as const;
  const t0 = Date.now() - 6 * 60_000;
  // Drop the guests' automatic greetings so the whole conversation fits the panel.
  useRoomStore.setState({ messages: [], unread: 0 });
  CHAT[p.lang].forEach(([who, text], i) => {
    const msg: ChatMessage = {
      id: `demo-${i}`,
      fromPeer: who === "self" ? "peer-self" : peerId(who),
      fromName: who === "self" ? "Steve_42" : names[who],
      text,
      sentAt: new Date(t0 + i * 70_000).toISOString(),
      viaRelay: false,
    };
    if (who === "self") useRoomStore.getState().addOwnMessage(msg);
    else bus.emit("chatMessage", msg);
  });
}
