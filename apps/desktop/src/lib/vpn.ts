// Pure helpers for the VPN screen (no React, no backend calls).
import type { VpnMode, VpnNode } from "../bindings";
import type { ValidationKey } from "./validation";

export const VPN_MODES = ["off", "launcherOnly", "launcherAndGame"] as const satisfies readonly VpnMode[];

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
