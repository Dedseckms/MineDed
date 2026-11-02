import { describe, expect, it } from "vitest";

import type { SocialAttachment } from "../../bindings";
import { saveFileName } from "./saveFileName";

const shot = (over: Partial<SocialAttachment> = {}): SocialAttachment => ({
  id: "a1",
  fileName: "shot.png",
  mime: "image/webp",
  size: 1024,
  url: "",
  thumbUrl: null,
  width: null,
  height: null,
  ...over,
});

describe("saveFileName", () => {
  it("keeps the original extension even when the stored mime is webp", () => {
    expect(saveFileName(shot())).toBe("shot.png");
    expect(saveFileName(shot({ fileName: "holiday.JPG" }))).toBe("holiday.JPG");
    expect(saveFileName(shot({ fileName: "anim.gif", mime: "image/png" }))).toBe("anim.gif");
  });

  it("adds .png to pictures without a known extension", () => {
    expect(saveFileName(shot({ fileName: "picture" }))).toBe("picture.png");
    expect(saveFileName(shot({ fileName: "", id: "abc" }))).toBe("abc.png");
  });

  it("keeps non-image files as they are and uses the mime only without a name", () => {
    expect(saveFileName(shot({ fileName: "world.zip", mime: "application/zip" }))).toBe("world.zip");
    expect(saveFileName(shot({ fileName: "", id: "f2", mime: "application/zip" }))).toBe("f2.zip");
  });

  it("strips characters a file system rejects", () => {
    expect(saveFileName(shot({ fileName: 'a:b?c.png' }))).toBe("a_b_c.png");
  });
});
