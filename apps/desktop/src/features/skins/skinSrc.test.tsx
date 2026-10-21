// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VirtualList } from "../../components/ui/VirtualList";
import { skinSrc } from "./skinSrc";

afterEach(cleanup);

describe("skinSrc", () => {
  const toAsset = (p: string) => `asset://localhost/${encodeURIComponent(p)}`;

  it("prefers the texture URL", () => {
    expect(skinSrc({ url: "https://t/skin.png", localPath: "C:\\s.png" }, toAsset)).toBe("https://t/skin.png");
  });

  it("falls back to the local file through the asset protocol", () => {
    expect(skinSrc({ url: "", localPath: "C:\\s.png" }, toAsset)).toBe(`asset://localhost/${encodeURIComponent("C:\\s.png")}`);
    expect(skinSrc({ url: " ", localPath: null }, toAsset)).toBeNull();
  });

  it("returns null when the asset protocol is unavailable", () => {
    const throwing = () => {
      throw new Error("no tauri");
    };
    expect(skinSrc({ url: "", localPath: "/s.png" }, throwing)).toBeNull();
  });
});

describe("VirtualList", () => {
  const items = Array.from({ length: 100 }, (_, i) => `row-${i}`);

  it("mounts only visible rows and reports the end", () => {
    const onEnd = vi.fn();
    render(
      <VirtualList items={items} rowHeight={50} height={200} getKey={(s) => s} renderRow={(s) => <span>{s}</span>} onEndReached={onEnd} />,
    );
    expect(screen.getAllByRole("listitem").length).toBeLessThan(15);
    expect(screen.queryByText("row-99")).toBeNull();
    expect(onEnd).not.toHaveBeenCalled();
    const list = screen.getByRole("list");
    list.scrollTop = 4800;
    fireEvent.scroll(list);
    expect(screen.getByText("row-99")).toBeTruthy();
    expect(onEnd).toHaveBeenCalled();
  });

  it("asks for more right away when a short list fits", () => {
    const onEnd = vi.fn();
    render(<VirtualList items={["a", "b"]} rowHeight={50} height={200} getKey={(s) => s} renderRow={(s) => s} onEndReached={onEnd} />);
    expect(onEnd).toHaveBeenCalledTimes(1);
  });
});
