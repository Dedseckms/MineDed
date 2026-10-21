import { type ReactNode, useState } from "react";

import { visibleRange } from "../../lib/content";

export interface VirtualListProps<T> {
  items: T[];
  rowHeight: number;
  /** Viewport height in px (the list scrolls inside it). */
  height: number;
  getKey: (item: T) => string;
  renderRow: (item: T, index: number) => ReactNode;
  ariaLabel?: string;
}

/** Fixed-row-height windowed list: only visible rows are mounted. */
export function VirtualList<T>({ items, rowHeight, height, getKey, renderRow, ariaLabel }: VirtualListProps<T>) {
  const [scrollTop, setScrollTop] = useState(0);
  const viewport = Math.min(height, items.length * rowHeight);
  const [start, end] = visibleRange(scrollTop, viewport || height, rowHeight, items.length);
  return (
    <div
      role="list"
      aria-label={ariaLabel}
      className="overflow-y-auto"
      style={{ height: viewport }}
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
