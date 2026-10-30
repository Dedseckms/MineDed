// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "../../i18n";
import { PhotoViewer } from "./PhotoViewer";

afterEach(cleanup);
// happy-dom has no Web Animations: motion falls back to instant presets.
for (const proto of [Element.prototype, HTMLElement.prototype])
  Reflect.deleteProperty(proto, "animate");

const SRC = "data:image/webp;base64,AAAA";

function mount(props: Partial<Parameters<typeof PhotoViewer>[0]> = {}) {
  const onClose = vi.fn();
  const onSave = vi.fn();
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={qc}>
      <PhotoViewer
        src={SRC}
        alt="screenshot"
        onClose={onClose}
        onSave={onSave}
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onClose, onSave };
}

describe("photo viewer", () => {
  it("shows the full-size photo over the app", () => {
    mount();
    expect(
      screen.getByRole("dialog", { name: i18n.t("social.photoViewer") }),
    ).toBeTruthy();
    const image = screen.getByRole("img", { name: "screenshot" });
    expect(image.getAttribute("src")).toBe(SRC);
    expect(image.className).toContain("object-contain");
    expect(
      screen.getByRole("button", { name: i18n.t("common.save") }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: i18n.t("common.close") }),
    ).toBeTruthy();
  });

  it("renders nothing while closed", () => {
    mount({ src: null });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes on Escape", () => {
    const { onClose } = mount();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on a backdrop click but not on the photo itself", () => {
    const { onClose } = mount();
    fireEvent.mouseDown(screen.getByRole("img", { name: "screenshot" }));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(screen.getByTestId("photo-backdrop"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("saves the original from the toolbar and keeps page scroll alone", () => {
    const { onSave } = mount();
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.save") }));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(document.body.style.overflow).toBe("");
  });

  it("moves focus to the close button when it opens", () => {
    mount();
    expect(document.activeElement?.textContent).toBe(
      i18n.t("common.close"),
    );
  });
});
