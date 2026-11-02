import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { SocialMessage } from "../../bindings";
import { Button } from "../../components/ui/Button";
import type { TimelineRow } from "./chat";
import { imageOnly, segmentsOf } from "./chat";

const OVERSCAN_PX = 600;
const FALLBACK_VIEWPORT = 480;
const NEAR_BOTTOM_PX = 120;
const LOAD_OLDER_PX = 200;

/** Rough bubble height; rows are measured after mount and corrected on scroll. */
function estimateMessage(message: SocialMessage): number {
  const content = message.content;
  if (content.type === "text") {
    const lines = content.text
      .split("\n")
      .reduce((n, line) => n + Math.max(1, Math.ceil(line.length / 40)), 0);
    return 26 + lines * 19;
  }
  if (content.type === "attachment") {
    if (imageOnly(message)) return 232;
    return content.caption ? 104 : 82;
  }
  if (content.type === "roomInvite") return 74;
  return 118;
}

/** Consecutive images share one grid bubble, so their cells are counted once. */
function estimateImages(count: number): number {
  const columns = count === 1 ? 1 : count <= 4 ? 2 : 3;
  return Math.ceil(count / columns) * (columns === 1 ? 236 : 150) + 8;
}

export function estimateRow(row: TimelineRow): number {
  if (row.kind === "day") return 34;
  let height = 6;
  for (const segment of segmentsOf(row.group.messages))
    height +=
      segment.kind === "images"
        ? estimateImages(segment.messages.length)
        : estimateMessage(segment.message) + 3;
  return height;
}

export interface ChatTimelineProps {
  rows: TimelineRow[];
  /** Resets the scroll position to the newest message when the channel changes. */
  channel: string;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  /** The viewport sits on the newest message (the reader is not browsing history). */
  onAtBottomChange?: (value: boolean) => void;
  renderRow: (row: TimelineRow) => ReactNode;
  empty: ReactNode;
}

/**
 * Windowed message list: only rows around the viewport are mounted, heights are
 * measured once and cached, older history loads when the top is reached.
 */
export function ChatTimeline({
  rows,
  channel,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  onAtBottomChange,
  renderRow,
  empty,
}: ChatTimelineProps) {
  const { t } = useTranslation();
  const scroller = useRef<HTMLDivElement>(null);
  const heights = useRef(new Map<string, number>());
  const previous = useRef({ height: 0, firstKey: "" });
  const atBottom = useRef(true);
  const bottomListener = useRef(onAtBottomChange);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(0);
  const [, setVersion] = useState(0);

  useEffect(() => {
    bottomListener.current = onAtBottomChange;
  }, [onAtBottomChange]);

  const setBottom = (value: boolean) => {
    if (atBottom.current === value) return;
    atBottom.current = value;
    bottomListener.current?.(value);
  };

  const measure = (key: string, value: number) => {
    if (value <= 0) return;
    const known = heights.current.get(key);
    if (known !== undefined && Math.abs(known - value) < 2) return;
    heights.current.set(key, value);
    setVersion((v) => v + 1);
  };

  const height = viewport || FALLBACK_VIEWPORT;
  const rowHeights = rows.map(
    (row) => heights.current.get(row.key) ?? estimateRow(row),
  );
  const total = rowHeights.reduce((sum, value) => sum + value, 0);
  const from = Math.max(0, scrollTop - OVERSCAN_PX);
  const to = scrollTop + height + OVERSCAN_PX;

  let start = rows.length;
  let end = rows.length;
  let top = 0;
  let topSpace = total;
  for (let i = 0; i < rows.length; i++) {
    const rowHeight = rowHeights[i] ?? 0;
    if (start === rows.length && top + rowHeight > from) {
      start = i;
      topSpace = top;
    }
    if (start !== rows.length && top > to) {
      end = i;
      break;
    }
    top += rowHeight;
  }
  const rendered = end > start ? rows.slice(start, end) : [];
  const renderedHeight = rendered.reduce(
    (sum, row) => sum + (heights.current.get(row.key) ?? estimateRow(row)),
    0,
  );
  const bottomSpace = Math.max(0, total - topSpace - renderedHeight);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setScrollTop(el.scrollTop);
    setBottom(true);
  }, [channel]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const firstKey = rows[0]?.key ?? "";
    if (previous.current.firstKey && previous.current.firstKey !== firstKey)
      el.scrollTop += el.scrollHeight - previous.current.height;
    previous.current = { height: el.scrollHeight, firstKey };
  }, [rows]);

  const lastKey = rows.at(-1)?.key ?? "";
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [lastKey]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const update = () => setViewport(el.clientHeight);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <div
      ref={scroller}
      role="log"
      aria-label={t("social.history")}
      className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain"
      onScroll={(e) => {
        const el = e.currentTarget;
        setScrollTop(el.scrollTop);
        setBottom(
          el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX,
        );
        if (el.scrollTop < LOAD_OLDER_PX && hasOlder && !loadingOlder)
          onLoadOlder();
      }}
    >
      {rows.length === 0 ? (
        <div className="grid h-full place-items-center p-6 text-muted">
          {empty}
        </div>
      ) : (
        <>
          {hasOlder || loadingOlder ? (
            <div className="flex justify-center py-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={loadingOlder}
                onClick={onLoadOlder}
              >
                {t(loadingOlder ? "common.loading" : "social.older")}
              </Button>
            </div>
          ) : null}
          <div aria-hidden style={{ height: topSpace }} />
          {rendered.map((row) => (
            <MeasuredRow key={row.key} id={row.key} measure={measure}>
              {renderRow(row)}
            </MeasuredRow>
          ))}
          <div aria-hidden style={{ height: bottomSpace }} />
        </>
      )}
    </div>
  );
}

function MeasuredRow({
  id,
  measure,
  children,
}: {
  id: string;
  measure: (key: string, value: number) => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    // The wrapper generates no box (`display: contents`): sticky date chips must
    // stay constrained by the scroll container, not by a one-row box.
    const el = ref.current?.firstElementChild;
    if (el instanceof HTMLElement) measure(id, el.offsetHeight);
  });
  return (
    <div ref={ref} style={{ display: "contents" }}>
      {children}
    </div>
  );
}
