// In-process event bus that stands in for Tauri events in mock mode.
import type { events } from "../bindings";

type Events = typeof events;
export type EventName = keyof Events;
export type EventPayload<N extends EventName> = Parameters<
  Parameters<Events[N]["listen"]>[0]
>[0]["payload"];

type Handler = (payload: unknown) => void;

export interface MockBus {
  on<N extends EventName>(name: N, cb: (payload: EventPayload<N>) => void): () => void;
  emit<N extends EventName>(name: N, payload: EventPayload<N>): void;
}

export function createMockBus(): MockBus {
  const handlers = new Map<EventName, Set<Handler>>();
  return {
    on(name, cb) {
      let set = handlers.get(name);
      if (!set) {
        set = new Set();
        handlers.set(name, set);
      }
      const h = cb as Handler;
      set.add(h);
      return () => {
        set.delete(h);
      };
    },
    emit(name, payload) {
      for (const h of [...(handlers.get(name) ?? [])]) h(payload);
    },
  };
}

/** Shared bus used by the app-wide mock backend. */
export const mockBus: MockBus = createMockBus();
