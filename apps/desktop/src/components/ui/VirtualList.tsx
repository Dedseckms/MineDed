import { type ReactNode, useEffect, useState } from "react";

import { visibleRange } from "../../lib/content";

export interface VirtualListProps<T> {
  items: T[];
  rowHeight: number;
  /** Viewport height in px (the list scrolls inside it). */
  height: number;
  getKey: (item: T) => string;
  renderRow: (item: T, index: number) => ReactNode;
  ariaLabel?: string;
  /** Called when the last rows come into view (infinite loading). */
  onEndReached?: () => void;
  /** Rows before the end that trigger `onEndReached`. */
  endThreshold?: number;
}

/** Fixed-row-height windowed list: only visible rows are mounted. */
export function VirtualList<T>({
  items,
  rowHeight,
  height,
  getKey,
  renderRow,
  ariaLabel,
  onEndReached,
  endThreshold = 3,
}: VirtualListProps<T>) {
  const [scrollTop, setScrollTop] = useState(0);
  const viewport = Math.min(height, items.length * rowHeight);
  const [start, end] = visibleRange(scrollTop, viewport || height, rowHeight, items.length);
  const lastVisible = Math.min(items.length, Math.ceil((scrollTop + (viewport || height)) / rowHeight));
  const nearEnd = items.length > 0 && lastVisible >= items.length - endThreshold;
  useEffect(() => {
    if (nearEnd) onEndReached?.();
  }, [nearEnd, items.length, onEndReached]);
  return (
    <div
      role="list"
      aria-label={ariaLabel}
      className="overflow-y-auto"
      // Never taller than ~70% of the window (1366x768): the page itself keeps scrolling.
      style={{ height: viewport, maxHeight: "70vh" }}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      <div style={{ height: items.length * rowHeight, position: "relative" }}>
        {items.slice(start, end).map((item, i) => (
          <div
            key={getKey(item)}
            role="listitem"
            style={{ position: "absolute", top: (start + i) * rowHeight, left: 0, right: 0, height: rowHeight }}
          >
            {renderRow(item, start + i)}
          </div>
        ))}
      </div>
    </div>
  );
}
