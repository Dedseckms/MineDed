// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { useContentStore } from "../../stores/contentStore";
import { Catalog } from "./Catalog";

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
