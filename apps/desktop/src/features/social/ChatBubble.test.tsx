// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Commands } from "../../api/client";
import type { SocialContent, SocialMessage } from "../../bindings";

const h = vi.hoisted(() => ({ api: null as unknown as Commands }));
vi.mock("../../api/client", async (orig) => {
  const actual = await orig<typeof import("../../api/client")>();
  return {
    ...actual,
    api: new Proxy({} as Commands, {
      get: (_target, prop) =>
        (h.api as unknown as Record<string | symbol, unknown>)[prop],
    }),
  };
});

const { createMockBus } = await import("../../mocks/bus");
const { createMockCommands } = await import("../../mocks");
const { SOCIAL_PEER } = await import("../../mocks/social");
const { MessageGroupView } = await import("./ChatBubble");
const { buildTimeline } = await import("./chat");

const i18n = (await import("../../i18n")).default;

const actions = {
  selfUuid: SOCIAL_PEER.uuid,
  canModerate: false,
  showAuthors: false,
  onOpenProfile: () => {},
  onDelete: () => {},
  onReport: () => {},
  onModerateDelete: () => {},
  onSave: () => {},
};

let seq = 0;
function message(content: SocialContent): SocialMessage {
  seq += 1;
  return {
    id: String(seq),
    channel: "global",
    author: SOCIAL_PEER,
    content,
    createdAt: new Date().toISOString(),
    editedAt: null,
    deleted: false,
  };
}

function image(id = "shot.png", mime = "image/png") {
  return message({
    type: "attachment",
    caption: null,
    attachment: {
      id,
      fileName: id,
      mime,
      size: 2048,
      url: "",
      thumbUrl: null,
      width: null,
      height: null,
    },
  });
}

function mount(messages: SocialMessage[]) {
  const group = buildTimeline(messages, SOCIAL_PEER.uuid).find(
    (row) => row.kind === "group",
  );
  if (group?.kind !== "group") throw new Error("expected a message group");
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MessageGroupView group={group.group} actions={actions} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  h.api = createMockCommands(
    {
      socialAttachmentPreview: async () => ({
        status: "ok",
        data: "data:image/webp;base64,AAAA",
      }),
    },
    { bus: createMockBus(), tickMs: 5 },
  );
});
afterEach(() => cleanup());

describe("compact bubble", () => {
  it("wraps a short word in a tight bubble with the time inside", () => {
    const { container } = mount([message({ type: "text", text: "ok" })]);
    const bubble = container.querySelector('[data-own="true"]') as HTMLElement;
    expect(bubble.className).toContain("py-1");
    expect(bubble.className).not.toContain("py-1.5");
    // Shrink-wraps the text instead of filling the row; long text caps at ~70%.
    expect(bubble.className).toContain("w-fit");
    expect(bubble.parentElement?.className).toContain("max-w-[70%]");
    expect(bubble.querySelector("time")?.textContent).toMatch(/\d\d:\d\d/);
  });

  it("floats the actions so they add no height", () => {
    const { container } = mount([message({ type: "text", text: "ok" })]);
    const bubble = container.querySelector('[data-own="true"]') as HTMLElement;
    const toolbar = bubble.querySelector(
      `[aria-label="${i18n.t("social.messageActions")}"]`,
    ) as HTMLElement;
    expect(toolbar).toBeTruthy();
    expect(toolbar.className).toContain("absolute");
  });
});

describe("image previews", () => {
  it("shows the picture right away without a save button in the bubble", async () => {
    mount([image()]);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: i18n.t("social.openFullscreen") }),
      ).toBeTruthy(),
    );
    expect(screen.getByAltText(i18n.t("social.attachment"))).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: i18n.t("common.save") }),
    ).toBeNull();
  });

  it("merges a run of pictures into one grid bubble", async () => {
    const { container } = mount([image("a.png"), image("b.png")]);
    await waitFor(() =>
      expect(
        screen.getAllByRole("button", {
          name: i18n.t("social.openFullscreen"),
        }),
      ).toHaveLength(2),
    );
    expect(container.querySelectorAll('[data-kind="images"]')).toHaveLength(1);
  });

  it("keeps the save button for files that are not pictures", () => {
    mount([
      message({
        type: "attachment",
        caption: null,
        attachment: {
          id: "pack.zip",
          fileName: "pack.zip",
          mime: "application/zip",
          size: 4096,
          url: "",
          thumbUrl: null,
          width: null,
          height: null,
        },
      }),
    ]);
    expect(
      screen.getByRole("button", { name: i18n.t("common.save") }),
    ).toBeTruthy();
  });
});
