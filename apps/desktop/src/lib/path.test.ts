import { describe, expect, it } from "vitest";

import { fileName } from "./path";

describe("fileName", () => {
  it("takes the last segment of any path", () => {
    expect(fileName("C:\\Users\\me\\Pictures\\skin.png")).toBe("skin.png");
    expect(fileName("/home/u/packs/Faithful 32x.zip")).toBe("Faithful 32x.zip");
    expect(fileName("/home/u/shaders/")).toBe("shaders");
    expect(fileName("plain.jar")).toBe("plain.jar");
    expect(fileName("")).toBe("");
    expect(fileName("data:image/png;base64,AAAA")).toBe("");
  });
});
