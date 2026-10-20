import { describe, expect, it } from "vitest";

import { createMockCommands } from "./index";
import { unwrap, ApiError } from "../api/client";

describe("mock commands", () => {
  it("returns overrides", async () => {
    const api = createMockCommands();
    expect(await api.appVersion()).toBe("0.0.0-mock");
    expect(unwrap(await api.instanceList())).toEqual([]);
  });

  it("falls back to notImplemented for every other command", async () => {
    const api = createMockCommands();
    const res = await api.vpnState();
    expect(res.status).toBe("error");
    expect(() => unwrap(res)).toThrow(ApiError);
  });

  it("accepts per-test overrides", async () => {
    const api = createMockCommands({ settingsSet: async () => ({ status: "ok", data: null }) });
    expect((await api.settingsSet({} as never)).status).toBe("ok");
  });
});
