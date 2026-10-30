import { beforeEach, describe, expect, it } from "vitest";
import { useSocialStore } from "./socialStore";
import { SOCIAL_PEER } from "../mocks/social";
import { applyTaskEvent } from "../lib/tasks";

describe("social session", () => {
  beforeEach(() => useSocialStore.getState().reset());
  it("clears the user, unread counter and blocks between accounts", () => {
    useSocialStore
      .getState()
      .set({
        self: SOCIAL_PEER,
        selected: "dm:a:b",
        unread: 8,
        connected: true,
      });
    useSocialStore.getState().block("peer", true);
    useSocialStore.getState().reset();
    expect(useSocialStore.getState()).toMatchObject({
      self: null,
      unread: 0,
      selected: "global",
      blocked: [],
      connected: false,
    });
  });
  it("blocks idempotently and can unblock", () => {
    const store = useSocialStore.getState();
    store.block("peer", true);
    store.block("peer", true);
    expect(useSocialStore.getState().blocked).toEqual(["peer"]);
    store.block("peer", false);
    expect(useSocialStore.getState().blocked).toEqual([]);
  });
  it("preserves transfer progress and reports failures rather than success", () => {
    const started = applyTaskEvent(undefined, {
      type: "started",
      id: "transfer",
      kind: "chatTransfer",
      titleKey: "tasks.chatSendFile",
    });
    const progressed = applyTaskEvent(started, {
      type: "progress",
      id: "transfer",
      done: 512,
      total: 1024,
      stage: "chat.upload",
      speedBps: 256,
    });
    expect(progressed).toMatchObject({
      kind: "chatTransfer",
      done: 512,
      total: 1024,
      status: "running",
    });
    const failed = applyTaskEvent(progressed, {
      type: "finished",
      id: "transfer",
      error: { code: "io", message: "disk full", details: null },
    });
    expect(failed.status).toBe("failed");
    expect(failed.speedBps).toBeNull();
  });
});
