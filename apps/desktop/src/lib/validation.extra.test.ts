import { describe, expect, it } from "vitest";

import {
  IMPORT_EXTS,
  SKIN_EXTS,
  skinNameFromPath,
  validateFilePath,
  validateOptionalUrl,
  validateRegister,
  validateSkinName,
} from "./validation";

describe("file paths", () => {
  it("requires an absolute path with an allowed extension", () => {
    expect(validateFilePath("", IMPORT_EXTS)).toBe("validation.required");
    expect(validateFilePath("pack.mrpack", IMPORT_EXTS)).toBe("validation.path.absolute");
    expect(validateFilePath("C:\\a\\pack.MRPACK", IMPORT_EXTS)).toBeNull();
    expect(validateFilePath("/home/u/pack.mrpack", IMPORT_EXTS)).toBeNull();
    expect(validateFilePath("/home/u/pack.zip", IMPORT_EXTS)).toBe("validation.path.ext");
    expect(validateFilePath("C:\\a\\pack.rar", IMPORT_EXTS)).toBe("validation.path.ext");
    expect(validateFilePath("C:\\a\\.png", SKIN_EXTS)).toBe("validation.path.ext");
    expect(validateFilePath("C:\\a\\b.png\\", SKIN_EXTS)).toBe("validation.path.ext");
    expect(validateFilePath("C:\\a\u0001.png", SKIN_EXTS)).toBe("validation.path.chars");
  });
});

describe("urls", () => {
  it("accepts empty or http(s) URLs", () => {
    expect(validateOptionalUrl("")).toBeNull();
    expect(validateOptionalUrl("https://auth.example.com/api/yggdrasil")).toBeNull();
    expect(validateOptionalUrl("auth.example.com")).toBe("validation.url.scheme");
    expect(validateOptionalUrl("ftp://x")).toBe("validation.url.scheme");
    expect(validateOptionalUrl("https://")).toBe("validation.url.invalid");
    expect(validateOptionalUrl("https://a b.com")).toBe("validation.url.invalid");
  });
});

describe("skins", () => {
  it("validates names and derives them from paths", () => {
    expect(validateSkinName(" ")).toBe("validation.required");
    expect(validateSkinName("x".repeat(33))).toBe("validation.skinName.length");
    expect(validateSkinName("Knight")).toBeNull();
    expect(skinNameFromPath("C:\\Users\\me\\Knight.PNG")).toBe("Knight");
    expect(skinNameFromPath("/a/b/steve.png")).toBe("steve");
  });
});

describe("registration", () => {
  const ok = { invite: "ABCD-1234", username: "New_Player", password: "secret123", confirm: "secret123" };
  it("accepts a valid draft", () => {
    expect(validateRegister(ok)).toEqual({});
  });
  it("reports every field", () => {
    expect(validateRegister({ invite: "", username: "a b", password: "short", confirm: "x" })).toEqual({
      invite: "validation.required",
      username: "validation.nick.chars",
      password: "validation.password.short",
      confirm: "validation.password.mismatch",
    });
    expect(validateRegister({ ...ok, invite: "bad code!" }).invite).toBe("validation.invite");
  });
});
