import { events } from "../bindings";
import type { EventName, EventPayload } from "../mocks/bus";
import { isMock, loadMocks } from "./client";

export type { EventName, EventPayload };

type Listenable<P> = {
  listen: (cb: (e: { payload: P }) => void) => Promise<() => void>;
};

/** Sync-unsubscribable wrapper around an async subscription. */
function deferred(start: () => Promise<() => void>): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void start()
    .then((u) => {
      if (disposed) u();
      else unlisten = u;
    })
    .catch(() => {
      /* event system unavailable; nothing to clean up */
    });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/**
 * Subscribes to a backend event (`events.*` from bindings, or the lazily
 * loaded mock bus). Returns a synchronous unsubscribe that is safe to call
 * before the subscription is established.
 */
export function subscribe<N extends EventName>(
  name: N,
  cb: (payload: EventPayload<N>) => void,
): () => void {
  if (isMock) return deferred(async () => (await loadMocks()).mockBus.on(name, cb));
  const ev = events[name] as unknown as Listenable<EventPayload<N>>;
  return deferred(() => ev.listen((e) => cb(e.payload)));
}
