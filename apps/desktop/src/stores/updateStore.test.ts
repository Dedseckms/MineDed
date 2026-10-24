import { beforeEach, describe, expect, it } from "vitest";

import { stageKey } from "../lib/tasks";
import { selectBannerUpdate, useUpdateStore } from "./updateStore";

const info = (version: string) => ({ version, currentVersion: "1.0.0", notes: "", date: null });

describe("updateStore", () => {
  beforeEach(() => useUpdateStore.getState().reset());

  it("banner shows the available update until postponed", () => {
    const s = useUpdateStore.getState();
    expect(selectBannerUpdate(useUpdateStore.getState())).toBeNull();
    s.setAvailable(info("1.0.1"));
    expect(selectBannerUpdate(useUpdateStore.getState())?.version).toBe("1.0.1");
    s.dismiss();
    expect(selectBannerUpdate(useUpdateStore.getState())).toBeNull();
  });

  it("a newer version reappears after Later; undismiss restores it", () => {
    const s = useUpdateStore.getState();
    s.setAvailable(info("1.0.1"));
    s.dismiss();
    s.setAvailable(info("1.0.2"));
    expect(selectBannerUpdate(useUpdateStore.getState())?.version).toBe("1.0.2");
    s.dismiss();
    s.undismiss();
    expect(selectBannerUpdate(useUpdateStore.getState())?.version).toBe("1.0.2");
  });

  it("up to date clears the update", () => {
    const s = useUpdateStore.getState();
    s.setAvailable(info("1.0.1"));
    s.setAvailable(null);
    expect(selectBannerUpdate(useUpdateStore.getState())).toBeNull();
  });
});

describe("stageKey", () => {
  const known = new Set(["stages.download", "stages.check"]);
  const exists = (k: string) => known.has(k);
  it("keeps full keys, scopes bare names, leaves unknown as is", () => {
    expect(stageKey("stages.download", exists)).toBe("stages.download");
    expect(stageKey("check", exists)).toBe("stages.check");
    expect(stageKey("weird", exists)).toBe("weird");
    expect(stageKey("", exists)).toBe("");
  });
});
