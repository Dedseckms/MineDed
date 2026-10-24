import { beforeEach, describe, expect, it } from "vitest";

import { DEFAULT_MODPACK_FILTERS } from "../lib/modpacks";
import { selectModpackFilters, useModpackStore } from "./modpackStore";
import { useNavStore } from "./navStore";

describe("modpackStore", () => {
  beforeEach(() => useModpackStore.getState().reset());

  it("updates and resets filters", () => {
    const s = useModpackStore.getState();
    s.setSource("curseForge");
    s.setText("atm");
    s.setMc("1.20.1");
    s.setLoader("forge");
    s.setSort("updated");
    expect(selectModpackFilters(useModpackStore.getState())).toEqual({
      source: "curseForge",
      text: "atm",
      mc: "1.20.1",
      loader: "forge",
      sort: "updated",
    });
    useModpackStore.getState().reset();
    expect(selectModpackFilters(useModpackStore.getState())).toEqual(DEFAULT_MODPACK_FILTERS);
  });
});

describe("navStore library tab", () => {
  it("keeps the library tab across instance pages", () => {
    const nav = useNavStore.getState();
    nav.setLibraryTab("catalog");
    nav.showInstance("inst-fabric");
    nav.showInstance(null);
    expect(useNavStore.getState().libraryTab).toBe("catalog");
    nav.setLibraryTab("mine");
    expect(useNavStore.getState().libraryTab).toBe("mine");
  });
});
