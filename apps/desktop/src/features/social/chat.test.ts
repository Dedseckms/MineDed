import { describe, expect, it } from "vitest";

import type {
  SocialHistory,
  SocialMessage,
  SocialUser,
} from "../../bindings";
import {
  GROUP_WINDOW_MS,
  buildTimeline,
  chronological,
  collapseText,
  dayLabel,
  previewOf,
} from "./chat";

const ALEX: SocialUser = {
  uuid: "alex",
  name: "Alex",
  skinUrl: null,
  status: "online",
  role: "user",
  isBlocked: false,
  isFriend: false,
  canMessage: true,
  canInvite: true,
};

const SELF: SocialUser = { ...ALEX, uuid: "self", name: "Me" };

function text(
  id: string,
  author: SocialUser,
  createdAt: string,
  body = "hi",
): SocialMessage {
  return {
    id,
    channel: "global",
    author,
    content: { type: "text", text: body },
    createdAt,
    editedAt: null,
    deleted: false,
  };
}

const today = new Date(2026, 9, 30, 12, 0, 0);

function at(day: number, hour: number, minute = 0): string {
  const d = new Date(today);
  d.setDate(d.getDate() - day);
  d.setHours(hour, minute, 0, 0);
  return d.toISOString();
}

describe("chat timeline", () => {
  it("orders pages oldest first and drops duplicate ids", () => {
    const older: SocialHistory = {
      messages: [text("1", ALEX, at(0, 9)), text("2", ALEX, at(0, 10))],
      before: null,
    };
    const newest: SocialHistory = {
      messages: [text("2", ALEX, at(0, 10)), text("3", SELF, at(0, 11))],
      before: "1",
    };
    // Pages arrive newest-first: [newest, older].
    expect(chronological([newest, older]).map((m) => m.id)).toEqual([
      "1",
      "2",
      "3",
    ]);
  });

  it("groups one author's quick messages and keeps others separate", () => {
    const rows = buildTimeline(
      [
        text("1", ALEX, at(0, 10, 0)),
        text("2", ALEX, at(0, 10, 2)),
        text("3", SELF, at(0, 10, 3)),
        text("4", ALEX, at(0, 10, 4)),
      ],
      SELF.uuid,
    );
    expect(rows[0]).toMatchObject({ kind: "day" });
    const groups = rows.filter((row) => row.kind === "group");
    expect(groups).toHaveLength(3);
    expect(groups.map((row) => row.kind === "group" && row.group.messages.map((m) => m.id))).toEqual([
      ["1", "2"],
      ["3"],
      ["4"],
    ]);
    expect(groups[0]).toMatchObject({ kind: "group", group: { own: false } });
    expect(groups[1]).toMatchObject({ kind: "group", group: { own: true } });
  });

  it("splits a group after the time window and at a new day", () => {
    const late = new Date(at(0, 10, 0));
    late.setTime(late.getTime() + GROUP_WINDOW_MS + 1000);
    const rows = buildTimeline(
      [
        text("1", ALEX, at(0, 10, 0)),
        text("2", ALEX, late.toISOString()),
        text("3", ALEX, at(1, 9, 0)),
      ],
      SELF.uuid,
    );
    expect(rows.map((row) => row.kind)).toEqual([
      "day",
      "group",
      "group",
      "day",
      "group",
    ]);
  });

  it("labels today, yesterday and older days", () => {
    expect(dayLabel(at(0, 8), today, "ru")).toEqual({ key: "social.date.today" });
    expect(dayLabel(at(1, 8), today, "ru")).toEqual({
      key: "social.date.yesterday",
    });
    const older = dayLabel(at(5, 8), today, "en");
    expect("date" in older ? older.date : "").not.toBe("");
  });

  it("collapses and trims previews", () => {
    expect(collapseText("  one\n two  ")).toBe("one two");
    const long = collapseText("x".repeat(400));
    expect(long.length).toBeLessThanOrEqual(120);
    expect(long.endsWith("…")).toBe(true);
  });

  it("describes every message kind for the chat list", () => {
    const attachment: SocialMessage = {
      ...text("a", ALEX, at(0, 10)),
      content: {
        type: "attachment",
        caption: null,
        attachment: {
          id: "f",
          fileName: "world.zip",
          mime: "application/zip",
          size: 10,
          url: "",
          thumbUrl: null,
          width: null,
          height: null,
        },
      },
    };
    const invite: SocialMessage = {
      ...text("i", ALEX, at(0, 10)),
      content: { type: "roomInvite", code: "ABCD-1234", link: "minedit://x" },
    };
    const pack: SocialMessage = {
      ...text("p", ALEX, at(0, 10)),
      content: {
        type: "packShare",
        name: "Skyblock",
        code: null,
        passwordProtected: false,
        mc: "1.21.1",
        loader: "fabric",
        mods: 3,
      },
    };
    expect(previewOf(null)).toBeNull();
    expect(previewOf(text("t", ALEX, at(0, 10), "hello"))).toEqual({
      kind: "text",
      text: "hello",
    });
    expect(previewOf(attachment)).toMatchObject({
      kind: "key",
      key: "social.previewFile",
      params: { name: "world.zip" },
    });
    expect(previewOf(invite)).toMatchObject({ key: "social.roomInvite" });
    expect(previewOf(pack)).toMatchObject({ key: "social.previewPack" });
    expect(previewOf({ ...text("d", ALEX, at(0, 10)), deleted: true })).toMatchObject({
      key: "social.deleted",
    });
  });
});
