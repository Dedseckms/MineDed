// Pure helpers for the VPN screen (no React, no backend calls).
import type { Account, VpnAccess, VpnApp, VpnMode, VpnNode } from "../bindings";
import type { ValidationKey } from "./validation";

export const VPN_MODES = ["off", "launcherOnly", "launcherAndGame", "apps"] as const satisfies readonly VpnMode[];

// ---- MineDed VPN -------------------------------------------------------------

/** How often the usage is re-read while the VPN screen is open. */
export const MINEDED_POLL_MS = 3 * 60_000;

const GIB = 1024 ** 3;

/** Bytes → GiB with at most one decimal (`12.5`), for "X GB left". */
export function toGb(bytes: number): number {
  return Math.max(0, Math.round((bytes / GIB) * 10) / 10);
}

export interface MinededUsage {
  /** 0..1 share of the quota already used. */
  used: number;
  leftBytes: number;
  /** Quota used up or the key is disabled. */
  exhausted: boolean;
}

export function minededUsage(a: VpnAccess): MinededUsage {
  const leftBytes = Math.max(0, a.quotaBytes - a.usedBytes);
  const used = a.quotaBytes > 0 ? Math.min(1, a.usedBytes / a.quotaBytes) : 1;
  return { used, leftBytes, exhausted: !a.enabled || leftBytes === 0 };
}

/** Own-server (Yggdrasil) accounts that can get MineDed VPN; the default one first. */
export function minededAccounts(accounts: readonly Account[]): Account[] {
  return accounts
    .filter((a) => a.kind === "yggdrasil")
    .sort((a, b) => Number(b.isDefault) - Number(a.isDefault));
}

/** Mode to enable after connecting: per-app routing when apps are chosen. */
export function minededMode(apps: readonly VpnApp[]): VpnMode {
  return apps.length > 0 ? "apps" : "launcherAndGame";
}

// ---- Apps mode ---------------------------------------------------------------

export interface AppPreset {
  name: string;
  windows: string;
  linux: string;
}

/** Popular programs with their process names per OS. */
export const APP_PRESETS: readonly AppPreset[] = [
  { name: "Telegram", windows: "Telegram.exe", linux: "telegram-desktop" },
  { name: "Discord", windows: "Discord.exe", linux: "Discord" },
  { name: "Chrome", windows: "chrome.exe", linux: "chrome" },
  { name: "Firefox", windows: "firefox.exe", linux: "firefox" },
  { name: "Edge", windows: "msedge.exe", linux: "msedge" },
  { name: "Yandex Browser", windows: "browser.exe", linux: "yandex_browser" },
  { name: "Opera", windows: "opera.exe", linux: "opera" },
  { name: "Steam", windows: "steam.exe", linux: "steam" },
];

/** Preset as a `VpnApp` for the given `SystemInfo.os`. */
export function presetApp(p: AppPreset, os: string): VpnApp {
  return { name: p.name, process: os === "linux" ? p.linux : p.windows, path: null };
}

const appKey = (a: Pick<VpnApp, "process">) => a.process.trim().toLowerCase();

export function hasApp(list: readonly VpnApp[], app: Pick<VpnApp, "process">): boolean {
  return list.some((a) => appKey(a) === appKey(app));
}

/** Appends unless the same process is already there (case-insensitive). */
export function addApp(list: readonly VpnApp[], app: VpnApp): VpnApp[] {
  return hasApp(list, app) || app.process.trim() === "" ? [...list] : [...list, app];
}

export function removeApp(list: readonly VpnApp[], process: string): VpnApp[] {
  return list.filter((a) => appKey(a) !== appKey({ process }));
}

/** App from a picked executable: process = file name, name = without `.exe`. */
export function appFromPath(path: string): VpnApp | null {
  const base = path.trim().split(/[\\/]/).pop() ?? "";
  if (base === "") return null;
  return { name: base.replace(/\.exe$/i, ""), process: base, path: path.trim() };
}

/** Case-insensitive filter by name or process. */
export function filterApps(list: readonly VpnApp[], query: string): VpnApp[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [...list];
  return list.filter((a) => a.name.toLowerCase().includes(q) || a.process.toLowerCase().includes(q));
}

export function sameApps(a: readonly VpnApp[], b: readonly VpnApp[]): boolean {
  return a.length === b.length && a.every((x, i) => appKey(x) === appKey(b[i] ?? { process: "" }));
}

/** Max subscription URLs accepted in one go (the backend keeps the list). */
export const SUBSCRIPTION_MAX = 16;

export type SubscriptionParse = { ok: true; urls: string[] } | { ok: false; error: ValidationKey };

/**
 * Splits the subscription text field (one URL per line, or separated by
 * spaces/commas) into unique http(s) URLs.
 */
export function parseSubscriptionUrls(text: string): SubscriptionParse {
  const parts = text
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  if (parts.length === 0) return { ok: false, error: "validation.required" };
  const urls: string[] = [];
  for (const p of parts) {
    if (!/^https?:\/\//i.test(p)) return { ok: false, error: "validation.url.scheme" };
    try {
      if (!new URL(p).hostname) return { ok: false, error: "validation.url.invalid" };
    } catch {
      return { ok: false, error: "validation.url.invalid" };
    }
    if (!urls.includes(p)) urls.push(p);
  }
  if (urls.length > SUBSCRIPTION_MAX) return { ok: false, error: "validation.vpn.tooMany" };
  return { ok: true, urls };
}

/** Absolute path to a local subscription file (any extension: yaml, txt, json…). */
export function validateSubscriptionPath(path: string): ValidationKey | null {
  const p = path.trim();
  if (p.length === 0) return "validation.required";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f]/.test(p)) return "validation.path.chars";
  if (!/^(?:[A-Za-z]:[\\/]|\\\\|\/|~[\\/])/.test(p)) return "validation.path.absolute";
  if (/[\\/]$/.test(p)) return "validation.path.ext";
  return null;
}

export type LatencyTone = "accent" | "warning" | "danger" | "neutral";

/** Colour of a latency badge: unknown → neutral, ≤150 ms good, ≤400 ms so-so. */
export function latencyTone(ms: number | null): LatencyTone {
  if (ms === null) return "neutral";
  if (ms <= 150) return "accent";
  if (ms <= 400) return "warning";
  return "danger";
}

export type NodeSort = "name" | "latency";

/** Sorted copy; by latency puts untested/timed-out nodes last, ties by name. */
export function sortNodes(nodes: readonly VpnNode[], by: NodeSort): VpnNode[] {
  const byName = (a: VpnNode, b: VpnNode) => a.name.localeCompare(b.name);
  if (by === "name") return [...nodes].sort(byName);
  return [...nodes].sort((a, b) => {
    if (a.latencyMs === null && b.latencyMs === null) return byName(a, b);
    if (a.latencyMs === null) return 1;
    if (b.latencyMs === null) return -1;
    return a.latencyMs - b.latencyMs || byName(a, b);
  });
}

/** Case-insensitive filter by name or protocol. */
export function filterNodes(nodes: readonly VpnNode[], query: string): VpnNode[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [...nodes];
  return nodes.filter((n) => n.name.toLowerCase().includes(q) || n.protocol.toLowerCase().includes(q));
}

/** Marks `name` as selected locally (optimistic update of `vpn_nodes`). */
export function selectNode(nodes: readonly VpnNode[], name: string): VpnNode[] {
  return nodes.map((n) => (n.selected === (n.name === name) ? n : { ...n, selected: n.name === name }));
}
