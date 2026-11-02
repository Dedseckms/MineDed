import { useEffect, useRef } from "react";

export interface AutoMarkReadOptions {
  /** Open channel id. */
  channel: string;
  /** Newest known message of the open channel (`null` = empty history). */
  latestId: string | null;
  /** The timeline sits on the newest message, i.e. the reader is not browsing. */
  pinned: boolean;
  markRead: (channel: string, messageId: string) => void;
  enabled?: boolean;
}

/**
 * Keeps the read marker on the newest message of the open channel: once when the
 * channel opens, whenever a new message arrives while the timeline is pinned to
 * the bottom, and once more when the reader leaves the channel. Scrolling up
 * stops the automatic marking until the bottom is reached again.
 */
export function useAutoMarkRead({
  channel,
  latestId,
  pinned,
  markRead,
  enabled = true,
}: AutoMarkReadOptions): void {
  const latest = useRef(new Map<string, string>());
  const marked = useRef(new Map<string, string>());
  const mark = useRef(markRead);

  useEffect(() => {
    mark.current = markRead;
  }, [markRead]);

  useEffect(() => {
    if (!enabled || !latestId) return;
    latest.current.set(channel, latestId);
    if (!pinned || marked.current.get(channel) === latestId) return;
    marked.current.set(channel, latestId);
    mark.current(channel, latestId);
  }, [channel, latestId, pinned, enabled]);

  useEffect(() => {
    if (!enabled) return;
    return () => {
      const id = latest.current.get(channel);
      if (!id || marked.current.get(channel) === id) return;
      marked.current.set(channel, id);
      mark.current(channel, id);
    };
  }, [channel, enabled]);
}
