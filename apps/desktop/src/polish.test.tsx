// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "./i18n";
import type { Settings } from "./bindings";
import { qk } from "./api/queries";
import { Modal } from "./components/ui/Modal";
import { ListSkeleton } from "./components/ui/feedback";
import { AccentPicker } from "./features/settings/SettingsScreen";
import { AppShell } from "./features/shell/AppShell";
import { applyAccent, useApplyAppearance } from "./hooks/useAppearance";
import i18n from "./i18n";
import { defaultSettings } from "./mocks/data";
import css from "./styles.css?raw";

afterEach(cleanup);

// happy-dom's WAAPI rejects `finished` on cancel; force Motion's JS path.
for (const proto of [Element.prototype, HTMLElement.prototype]) Reflect.deleteProperty(proto, "animate");

function wrapper(settings: Settings) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  qc.setQueryData(qk.settings, settings);
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("accent colour", () => {
  it("applyAccent sets and clears CSS variables", () => {
    const el = document.createElement("div");
    applyAccent(el, "#3b82f6", "dark");
    expect(el.style.getPropertyValue("--color-accent")).toMatch(/^#[0-9a-f]{6}$/);
    expect(el.style.getPropertyValue("--color-accent-fg")).toMatch(/^#(000000|ffffff)$/);
    applyAccent(el, null, "dark");
    expect(el.style.getPropertyValue("--color-accent")).toBe("");
    applyAccent(el, "garbage", "light");
    expect(el.style.getPropertyValue("--color-accent-strong")).toBe("");
  });

  it("useApplyAppearance applies Settings.accentColor to <html>", async () => {
    renderHook(() => useApplyAppearance(), { wrapper: wrapper({ ...defaultSettings(), theme: "dark", accentColor: "#4ade80" }) });
    await waitFor(() => expect(document.documentElement.style.getPropertyValue("--color-accent")).toBe("#4ade80"));
  });

  it("AccentPicker commits a swatch after a short pause and can reset", () => {
    vi.useFakeTimers();
    try {
      const onChange = vi.fn();
      const { rerender } = render(<AccentPicker value={null} onChange={onChange} />);
      const radios = screen.getAllByRole("radio");
      expect(radios[0]?.getAttribute("aria-checked")).toBe("true");
      fireEvent.click(radios[2] as HTMLElement);
      expect(onChange).not.toHaveBeenCalled();
      act(() => void vi.advanceTimersByTime(300));
      expect(onChange).toHaveBeenCalledWith("#38bdf8");
      rerender(<AccentPicker value="#38bdf8" onChange={onChange} />);
      expect(screen.getAllByRole("radio")[2]?.getAttribute("aria-checked")).toBe("true");
      fireEvent.click(screen.getAllByRole("radio")[0] as HTMLElement);
      act(() => void vi.advanceTimersByTime(300));
      expect(onChange).toHaveBeenLastCalledWith(null);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("light mode and reduced motion", () => {
  it("CSS disables animations, transitions and backdrop-filter in light mode", () => {
    const block = css.slice(css.indexOf('[data-light-mode="true"] *'));
    for (const rule of ["backdrop-filter: none", "animation: none", "transition: none"]) expect(block).toContain(rule);
    expect(css).toContain('[data-reduce-motion="true"] *');
  });

  it("modal has no backdrop blur in light mode", async () => {
    const W = wrapper({ ...defaultSettings(), lightMode: true });
    render(
      <W>
        <Modal open onClose={() => {}} title="t">
          <p>x</p>
        </Modal>
      </W>,
    );
    const dialog = await screen.findByRole("dialog");
    expect(dialog.parentElement?.className).not.toContain("backdrop-blur");
  });

  it("modal blurs the backdrop with full effects", async () => {
    const W = wrapper(defaultSettings());
    render(
      <W>
        <Modal open onClose={() => {}} title="t">
          <p>x</p>
        </Modal>
      </W>,
    );
    expect((await screen.findByRole("dialog")).parentElement?.className).toContain("backdrop-blur");
  });
});

describe("layout", () => {
  it("skeleton is announced as loading", async () => {
    await i18n.changeLanguage("en");
    render(<ListSkeleton rows={3} />);
    const status = screen.getByRole("status", { name: "Loading…" });
    expect(status.children).toHaveLength(3);
  });

  it("sidebar collapses to icons below lg and content is width-capped", async () => {
    await i18n.changeLanguage("en");
    render(<AppShell />, { wrapper: wrapper({ ...defaultSettings(), language: "en" }) });
    const nav = screen.getByTestId("sidebar");
    expect(nav.className).toMatch(/\bw-16\b/);
    expect(nav.className).toMatch(/\blg:w-56\b/);
    // Labels stay accessible when collapsed.
    const item = screen.getByRole("button", { name: "Settings" });
    expect(item.getAttribute("title")).toBe("Settings");
    const main = screen.getByTestId("main");
    expect(main.className).toContain("overflow-x-hidden");
    expect(main.firstElementChild?.className).toMatch(/max-w-5xl/);
  });
});
