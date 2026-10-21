// Realistic seed data for the mock backend.
import type {
  Account,
  Instance,
  Settings,
  SystemInfo,
  VersionEntry,
  VersionKind,
  YggdrasilServer,
} from "../bindings";

function javaFor(id: string): number {
  const [, minor = 0, patch = 0] = id.split(".").map((n) => Number(n));
  if (minor > 20 || (minor === 20 && patch >= 5)) return 21;
  if (minor >= 18) return 17;
  if (minor === 17) return 16;
  return 8;
}

const RELEASES: Array<[string, string]> = [
  ["1.21.10", "2025-10-07"],
  ["1.21.9", "2025-09-30"],
  ["1.21.8", "2025-07-17"],
  ["1.21.7", "2025-06-30"],
  ["1.21.6", "2025-06-17"],
  ["1.21.5", "2025-03-25"],
  ["1.21.4", "2024-12-03"],
  ["1.21.3", "2024-10-23"],
  ["1.21.1", "2024-08-08"],
  ["1.21", "2024-06-13"],
  ["1.20.6", "2024-04-29"],
  ["1.20.4", "2023-12-07"],
  ["1.20.2", "2023-09-21"],
  ["1.20.1", "2023-06-12"],
  ["1.19.4", "2023-03-14"],
  ["1.19.2", "2022-08-05"],
  ["1.18.2", "2022-02-28"],
  ["1.17.1", "2021-07-06"],
  ["1.16.5", "2021-01-15"],
  ["1.15.2", "2020-01-17"],
  ["1.14.4", "2019-07-19"],
  ["1.12.2", "2017-09-18"],
  ["1.8.9", "2015-12-09"],
  ["1.7.10", "2014-06-26"],
];

const SNAPSHOTS: Array<[string, string]> = [
  ["25w41a", "2025-10-08"],
  ["1.21.9-rc1", "2025-09-24"],
  ["25w37a", "2025-09-09"],
  ["1.21.6-pre1", "2025-06-03"],
  ["25w21a", "2025-05-20"],
  ["24w14a", "2024-04-03"],
];

function entry(id: string, date: string, kind: VersionKind, javaMajor: number | null): VersionEntry {
  return { id, kind, releaseTime: `${date}T10:00:00Z`, javaMajor };
}

export const VERSIONS: VersionEntry[] = [
  ...RELEASES.map(([id, d]) => entry(id, d, "release", javaFor(id))),
  ...SNAPSHOTS.map(([id, d]) => entry(id, d, "snapshot", 21)),
  entry("b1.7.3", "2011-07-08", "oldBeta", 8),
  entry("a1.2.6", "2010-12-03", "oldAlpha", 8),
].sort((a, b) => b.releaseTime.localeCompare(a.releaseTime));

const defaultMemory = { minMb: 1024, maxMb: 4096, auto: true };

export function seedInstances(): Instance[] {
  return [
    {
      id: "inst-survival",
      name: "Выживание 1.21",
      mc: "1.21.10",
      loader: { kind: "vanilla", version: null },
      java: { path: null, major: 21 },
      memory: { ...defaultMemory },
      jvmArgs: [],
      preset: "balanced",
      icon: null,
      createdAt: "2026-09-01T18:20:00Z",
      lastPlayed: "2026-10-19T21:05:00Z",
      playtimeSec: 86_400 + 3_600 * 5,
    },
    {
      id: "inst-classic",
      name: "Классика 1.12.2",
      mc: "1.12.2",
      loader: { kind: "vanilla", version: null },
      java: { path: null, major: 8 },
      memory: { minMb: 512, maxMb: 2048, auto: false },
      jvmArgs: ["-XX:+UseG1GC"],
      preset: null,
      icon: null,
      createdAt: "2026-07-14T12:00:00Z",
      lastPlayed: "2026-08-02T15:30:00Z",
      playtimeSec: 3_600 * 12,
    },
    {
      id: "inst-snapshot",
      name: "Снапшоты",
      mc: "25w41a",
      loader: { kind: "vanilla", version: null },
      java: { path: null, major: 21 },
      memory: { ...defaultMemory },
      jvmArgs: [],
      preset: null,
      icon: null,
      createdAt: "2026-10-10T09:00:00Z",
      lastPlayed: null,
      playtimeSec: 0,
    },
  ];
}

export function seedAccounts(): Account[] {
  return [
    {
      id: "acc-steve",
      kind: "offline",
      name: "Steve_42",
      uuid: "8667ba71-b85a-4004-af54-457a9734eed7",
      server: null,
      skinUrl: null,
      isDefault: true,
      valid: true,
    },
    {
      id: "acc-alex",
      kind: "offline",
      name: "AlexBuilder",
      uuid: "ec561538-f3fd-461d-aff5-086b22154bce",
      server: null,
      skinUrl: null,
      isDefault: false,
      valid: true,
    },
  ];
}

export const YGGDRASIL_SERVERS: YggdrasilServer[] = [
  { name: "MineUP", apiRoot: "https://auth.mineup.invalid/api/yggdrasil", official: true },
];

export const SYSTEM_INFO: SystemInfo = {
  os: "windows",
  osVersion: "11 Pro 26300",
  arch: "x86_64",
  cpuModel: "AMD Ryzen 5 5600X",
  cpuThreads: 12,
  totalRamMb: 16_384,
  freeRamMb: 9_870,
  gpu: "NVIDIA GeForce RTX 3060",
  weakHardware: false,
  wayland: false,
};

export function defaultSettings(): Settings {
  return {
    language: "ru",
    theme: "system",
    lightMode: false,
    reduceMotion: false,
    gameDir: null,
    defaultMemory: { minMb: 512, maxMb: 4096, auto: true },
    javaPath: null,
    defaultJvmArgs: [],
    maxParallelDownloads: 16,
    preferMirrors: false,
    proxy: null,
    closeOnLaunch: false,
    autoUpdate: true,
    hubUrl: "",
    skinServer: null,
  };
}

export const GAME_LOG_SAMPLE = [
  "[Render thread/INFO]: Backend library: LWJGL version 3.3.3",
  "[Render thread/INFO]: Reloading ResourceManager: vanilla",
  "[Worker-Main-3/INFO]: Found unifont_all_no_pua-15.1.05.hex, loading",
  "[Render thread/INFO]: Sound engine started",
  "[Render thread/INFO]: Created: 1024x512x4 minecraft:textures/atlas/blocks.png-atlas",
  "[Server thread/INFO]: Starting integrated minecraft server",
  "[Server thread/INFO]: Preparing start region for dimension minecraft:overworld",
  "[Server thread/INFO]: Time elapsed: 2143 ms",
  "[Server thread/INFO]: Player joined the game",
  "[Render thread/INFO]: Loaded 1287 advancements",
];
