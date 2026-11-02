// @vitest-environment happy-dom
import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useAutoMarkRead } from "./useAutoMarkRead";

type Props = {
  channel: string;
  latestId: string | null;
  pinned: boolean;
  enabled?: boolean;
};

function setup(initial: Props) {
  const markRead = vi.fn();
  const view = renderHook((props: Props) => useAutoMarkRead({ ...props, markRead }), {
    initialProps: initial,
  });
  return { markRead, view };
}

describe("useAutoMarkRead", () => {
  it("marks the open channel read as soon as the newest message is known", () => {
    const { markRead } = setup({ channel: "global", latestId: "1", pinned: true });
    expect(markRead).toHaveBeenCalledWith("global", "1");
  });

  it("marks once per message, not on every render", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: true,
    });
    view.rerender({ channel: "global", latestId: "1", pinned: true });
    expect(markRead).toHaveBeenCalledTimes(1);
  });

  it("stays quiet while the reader browses history", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: false,
    });
    expect(markRead).not.toHaveBeenCalled();
    view.rerender({ channel: "global", latestId: "2", pinned: false });
    expect(markRead).not.toHaveBeenCalled();
  });

  it("marks the newest message once the timeline is back at the bottom", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: false,
    });
    view.rerender({ channel: "global", latestId: "2", pinned: true });
    expect(markRead).toHaveBeenCalledWith("global", "2");
  });

  it("marks a new message that arrives in the open channel", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: true,
    });
    view.rerender({ channel: "global", latestId: "2", pinned: true });
    expect(markRead).toHaveBeenLastCalledWith("global", "2");
  });

  it("marks the channel read when the reader leaves it", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: false,
    });
    expect(markRead).not.toHaveBeenCalled();
    view.rerender({ channel: "dm:alex", latestId: null, pinned: true });
    expect(markRead).toHaveBeenCalledWith("global", "1");
    expect(markRead).toHaveBeenCalledTimes(1);
  });

  it("does nothing without a signed-in session", () => {
    const { markRead, view } = setup({
      channel: "global",
      latestId: "1",
      pinned: true,
      enabled: false,
    });
    view.rerender({ channel: "dm:alex", latestId: "2", pinned: true, enabled: false });
    expect(markRead).not.toHaveBeenCalled();
  });
});
