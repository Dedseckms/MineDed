// Pure helpers for the Telegram-style chat layout: ordering, grouping, labels.
import type { SocialHistory, SocialMessage } from "../../bindings";

/** Consecutive messages of one author within this window share a bubble group. */
export const GROUP_WINDOW_MS = 5 * 60 * 1000;

const PREVIEW_LENGTH = 120;

export interface MessageGroup {
  key: string;
  author: SocialMessage["author"];
  /** Written by the signed-in player (right-hand, accent bubble). */
  own: boolean;
  messages: SocialMessage[];
}

export type TimelineRow =
  | { kind: "day"; key: string; at: number }
  | { kind: "group"; key: string; group: MessageGroup };

export type DayLabel =
  | { key: "social.date.today" | "social.date.yesterday" }
  | { date: string };

/** Chat list preview: raw text or an i18n key with interpolation values. */
export type Preview =
  | { kind: "text"; text: string }
  | { kind: "key"; key: string; params: Record<string, string> };

function dayStart(value: Date): number {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
}

function dayKey(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** Collapses whitespace so a multi-line message fits one list line. */
export function collapseText(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > PREVIEW_LENGTH
    ? `${flat.slice(0, PREVIEW_LENGTH - 1)}…`
    : flat;
}

/**
 * Oldest page first, messages oldest → newest, one entry per id
 * (an edited copy from a newer page wins). `history` pages arrive newest-first.
 */
export function chronological(pages: SocialHistory[]): SocialMessage[] {
  const byId = new Map<string, SocialMessage>();
  for (const page of [...pages].reverse())
    for (const message of page.messages) byId.set(message.id, message);
  return [...byId.values()].sort(
    (a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt),
  );
}

/**
 * Day separators plus author groups: same author, same day and no more than
 * `GROUP_WINDOW_MS` between consecutive messages.
 */
export function buildTimeline(
  messages: SocialMessage[],
  selfUuid: string,
): TimelineRow[] {
  const rows: TimelineRow[] = [];
  let day = "";
  let group: MessageGroup | null = null;
  let previousAt = 0;
  for (const message of messages) {
    const at = Date.parse(message.createdAt);
    const currentDay = dayKey(message.createdAt);
    if (currentDay !== day) {
      day = currentDay;
      group = null;
      rows.push({ kind: "day", key: `day-${currentDay}`, at });
    }
    if (
      group &&
      group.author.uuid === message.author.uuid &&
      at - previousAt <= GROUP_WINDOW_MS
    ) {
      group.messages.push(message);
    } else {
      group = {
        key: `group-${message.id}`,
        author: message.author,
        own: message.author.uuid === selfUuid,
        messages: [message],
      };
      rows.push({ kind: "group", key: group.key, group });
    }
    previousAt = at;
  }
  return rows;
}

/** "Today" / "Yesterday" keys or a locale date for older days. */
export function dayLabel(value: string | number, now: Date, locale: string): DayLabel {
  const at = dayStart(new Date(value));
  if (at === dayStart(now)) return { key: "social.date.today" };
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (at === dayStart(yesterday)) return { key: "social.date.yesterday" };
  const date = new Date(value);
  return {
    date: date.toLocaleDateString(locale, {
      day: "numeric",
      month: "long",
      year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
    }),
  };
}

export function clockLabel(iso: string, locale: string): string {
  return new Date(iso).toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function previewOf(message: SocialMessage | null): Preview | null {
  if (!message) return null;
  if (message.deleted)
    return { kind: "key", key: "social.deleted", params: {} };
  const content = message.content;
  if (content.type === "text")
    return { kind: "text", text: collapseText(content.text) };
  if (content.type === "attachment")
    return {
      kind: "key",
      key: "social.previewFile",
      params: { name: content.attachment.fileName },
    };
  if (content.type === "roomInvite")
    return {
      kind: "key",
      key: "social.roomInvite",
      params: { code: content.code },
    };
  return { kind: "key", key: "social.previewPack", params: { name: content.name } };
}
