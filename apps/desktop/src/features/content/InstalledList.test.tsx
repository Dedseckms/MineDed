// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { InstalledContent, Instance } from "../../bindings";
import { api, unwrap } from "../../api/client";
import i18n from "../../i18n";
import { InstalledList } from "./InstalledList";

afterEach(cleanup);
beforeEach(async () => {
  await i18n.changeLanguage("en");
});

const item = (over: Partial<InstalledContent>): InstalledContent => ({
  fileName: "file.jar",
  kind: "mod",
  enabled: true,
  source: "modrinth",
  projectId: null,
  versionId: null,
  title: null,
  versionNumber: null,
  updateAvailable: false,
  ...over,
});

let instance: Instance;
async function renderList(items: InstalledContent[]) {
  instance ??= unwrap(await api.instanceList()).find((i) => i.id === "inst-fabric")!;
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <InstalledList instance={instance} kind="mod" items={items} onBrowse={() => {}} onInstall={() => {}} />
    </QueryClientProvider>,
  );
}

/** The list row holding `fileName` (the file name appears once inside the row). */
function rowOf(fileName: string): ReturnType<typeof within> {
  const text = screen
    .getAllByText((_, el) => el?.tagName === "P" && el.textContent?.includes(fileName) === true)
    .at(-1);
  const row = text?.closest(".border-b") as HTMLElement;
  return within(row);
}

describe("installed content rows", () => {
  it("shows where each file came from", async () => {
    await renderList([
      item({ fileName: "a.jar", source: "modrinth", projectId: "AANobbMI" }),
      item({ fileName: "b.jar", source: "curseForge", projectId: "306612" }),
      item({ fileName: "c.jar", source: "local", projectId: null }),
    ]);
    expect(rowOf("a.jar").getByText("Modrinth")).toBeTruthy();
    expect(rowOf("b.jar").getByText("CurseForge")).toBeTruthy();
    expect(rowOf("c.jar").getByText("Local")).toBeTruthy();
  });

  it("links provider files to their page and local ones have no link", async () => {
    await renderList([
      item({ fileName: "a.jar", source: "modrinth", projectId: "AANobbMI" }),
      item({ fileName: "b.jar", source: "curseForge", projectId: "306612" }),
      item({ fileName: "c.jar", source: "local", projectId: null }),
    ]);
    const link = (name: string) =>
      rowOf(name).getByTestId("project-page-link") as HTMLAnchorElement;
    expect(link("a.jar").href).toBe("https://modrinth.com/project/AANobbMI");
    expect(link("b.jar").href).toBe("https://www.curseforge.com/minecraft/mc-mods/306612");
    expect(link("a.jar").getAttribute("target")).toBe("_blank");
    expect(link("a.jar").getAttribute("rel")).toContain("noopener");
    expect(rowOf("c.jar").queryByTestId("project-page-link")).toBeNull();
  });
});
