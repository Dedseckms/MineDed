// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { useContentStore } from "../../stores/contentStore";
import { Catalog } from "./Catalog";

const opener = vi.hoisted(() => ({ openUrl: vi.fn(async (_url: string) => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

afterEach(cleanup);
beforeEach(async () => { useContentStore.getState().reset(); await i18n.changeLanguage("en"); });
it("searches and passes the actual selected provider to install and details", async () => {
  const instance = unwrap(await api.instanceList()).find((i) => i.id === "inst-fabric")!;
  const onOpen = vi.fn();
  const onInstall = vi.fn();
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
    <Catalog instance={instance} installed={[]} onOpen={onOpen} onInstall={onInstall} />
  </QueryClientProvider>);
  fireEvent.change(screen.getByRole("combobox", { name: "Provider" }), { target: { value: "curseForge" } });
  fireEvent.click(await screen.findByRole("button", { name: "Fabric API (CurseForge)" }));
  expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ source: "curseForge", projectId: "306612" }));
  fireEvent.click(screen.getByRole("button", { name: "Install" }));
  await waitFor(() => expect(onInstall).toHaveBeenCalledWith(expect.objectContaining({ source: "curseForge", projectId: "306612" })));
});

it("links every result to its page on the matching site", async () => {
  const instance = unwrap(await api.instanceList()).find((i) => i.id === "inst-fabric")!;
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
    <Catalog instance={instance} installed={[]} onOpen={() => {}} onInstall={() => {}} />
  </QueryClientProvider>);
  const opened: string[] = [];
  const modrinth = await screen.findAllByTestId("project-page-link");
  for (const link of modrinth) {
    fireEvent.click(link);
    opened.push(opener.openUrl.mock.calls.at(-1)?.[0] ?? "");
  }
  expect(opened.some((href) => href.startsWith("https://modrinth.com/project/"))).toBe(true);
  for (const href of opened) expect(href).not.toContain("/minecraft/");

  fireEvent.change(screen.getByRole("combobox", { name: "Provider" }), { target: { value: "curseForge" } });
  await waitFor(() => {
    const links = screen.getAllByTestId("project-page-link");
    expect(links.length).toBeGreaterThan(0);
    opener.openUrl.mockClear();
    for (const link of links) fireEvent.click(link);
    const hrefs = opener.openUrl.mock.calls.map(([href]) => href);
    expect(hrefs).toHaveLength(links.length);
    for (const href of hrefs) expect(href).toMatch(/^https:\/\/www\.curseforge\.com\/minecraft\/mc-mods\//);
  });
});
