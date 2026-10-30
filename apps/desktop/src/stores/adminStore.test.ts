import { beforeEach, describe, expect, it } from "vitest";

import type { SocialReport } from "../bindings";
import { useAdminStore } from "./adminStore";

const report = (id: string): SocialReport => ({
  id,
  message: {
    id: `m-${id}`,
    channel: "global",
    author: {
      uuid: "2".repeat(32),
      name: "Alex",
      skinUrl: null,
      status: "online",
      role: "user",
      isFriend: false,
      canMessage: true,
      canInvite: true,
    },
    content: { type: "text", text: "spam" },
    createdAt: new Date().toISOString(),
    editedAt: null,
    deleted: false,
  },
  reporter: {
    uuid: "3".repeat(32),
    name: "Steve",
    skinUrl: null,
    status: "online",
    role: "user",
    isFriend: false,
    canMessage: true,
    canInvite: true,
  },
  reason: "spam",
  createdAt: new Date().toISOString(),
});

beforeEach(() => useAdminStore.getState().reset());

describe("admin store", () => {
  it("keeps reports resolved in this session, newest first", () => {
    useAdminStore.getState().addResolved(report("1"));
    useAdminStore.getState().addResolved(report("2"));
    expect(useAdminStore.getState().resolved.map((r) => r.id)).toEqual([
      "2",
      "1",
    ]);
  });

  it("does not duplicate the same report", () => {
    useAdminStore.getState().addResolved(report("1"));
    useAdminStore.getState().addResolved(report("1"));
    expect(useAdminStore.getState().resolved).toHaveLength(1);
  });
});
