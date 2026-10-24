import { beforeEach, describe, expect, it } from "vitest";

import { selectFilters, useContentStore } from "./contentStore";
import { useNavStore } from "./navStore";

describe("contentStore", () => {
  beforeEach(() => useContentStore.getState().reset());

  it("defaults to compatible Modrinth mods", () => {
    expect(selectFilters(useContentStore.getState())).toEqual({
      source: "modrinth",
      kind: "mod",
      text: "",
      sort: "relevance",
      compatibleOnly: true,
    });
  });

  it("browse() switches kind and resets text and compatibility", () => {
    const s = useContentStore.getState();
    s.setText("iris");
    s.setCompatibleOnly(false);
    s.browse("shader");
    expect(selectFilters(useContentStore.getState())).toMatchObject({
      source: "modrinth",
      kind: "shader",
      text: "",
      compatibleOnly: true,
    });
  });
});

describe("navStore instance page", () => {
  it("opens an instance on the requested tab and closes on section change", () => {
    const nav = useNavStore.getState();
    nav.showInstance("inst-fabric", "shaders");
    expect(useNavStore.getState()).toMatchObject({ section: "instances", openInstance: "inst-fabric", instanceTab: "shaders" });
    useNavStore.getState().showInstance("inst-fabric");
    expect(useNavStore.getState().instanceTab).toBe("shaders");
    useNavStore.getState().showInstance("inst-big");
    expect(useNavStore.getState().instanceTab).toBe("mods");
    useNavStore.getState().go("skins");
    expect(useNavStore.getState().openInstance).toBeNull();
  });
});
