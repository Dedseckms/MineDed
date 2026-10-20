import { events } from "../bindings";
import { type EventName, type EventPayload, mockBus } from "../mocks/bus";
import { isMock } from "./client";

export type { EventName, EventPayload };

type Listenable<P> = {
  listen: (cb: (e: { payload: P }) => void) => Promise<() => void>;
};

/**
 * Subscribes to a backend event (`events.*` from bindings, or the mock bus).
 * Returns a synchronous unsubscribe that is safe to call before `listen` resolves.
 */
export function subscribe<N extends EventName>(
  name: N,
  cb: (payload: EventPayload<N>) => void,
): () => void {
  if (isMock) return mockBus.on(name, cb);
  let unlisten: (() => void) | null = null;
  let disposed = false;
  const ev = events[name] as unknown as Listenable<EventPayload<N>>;
  void ev
    .listen((e) => cb(e.payload))
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
