import { Channel } from "@tauri-apps/api/core";

import type { TaskEvent } from "../bindings";
import { isMock } from "./client";

/**
 * Creates the `Channel<TaskEvent>` passed to task commands. Outside Tauri the
 * real `Channel` cannot be constructed, so mocks get a duck-typed stand-in
 * (they only call `onmessage`).
 */
export function createTaskChannel(onEvent: (ev: TaskEvent) => void): Channel<TaskEvent> {
  if (!isMock) return new Channel<TaskEvent>(onEvent);
  return { id: -1, onmessage: onEvent } as unknown as Channel<TaskEvent>;
}
