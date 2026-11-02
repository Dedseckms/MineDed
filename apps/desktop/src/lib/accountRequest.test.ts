// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";

import {
  ACCOUNT_NOTE_MAX,
  createAccountFailure,
  hasRequestedAccount,
  markAccountRequested,
  normalizeNote,
  validateAccountLogin,
  validateAccountPassword,
} from "./accountRequest";

afterEach(() => localStorage.clear());

describe("account requests", () => {
  it("remembers a request per guest uuid", () => {
    expect(hasRequestedAccount("a")).toBe(false);
    markAccountRequested("a");
    expect(hasRequestedAccount("a")).toBe(true);
    expect(hasRequestedAccount("b")).toBe(false);
    expect(hasRequestedAccount("")).toBe(false);
  });

  it("survives a storage that throws", () => {
    const original = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("denied");
    };
    try {
      expect(hasRequestedAccount("a")).toBe(false);
      expect(() => markAccountRequested("a")).not.toThrow();
    } finally {
      Storage.prototype.getItem = original;
    }
  });

  it("checks the login and the password of a request", () => {
    expect(validateAccountLogin("ab")).toBe("validation.nick.length");
    expect(validateAccountLogin("bad nick")).toBe("validation.nick.chars");
    expect(validateAccountLogin("Steve_01")).toBeNull();
    expect(validateAccountPassword("")).toBe("validation.required");
    expect(validateAccountPassword("12345")).toBe("validation.password.range");
    expect(validateAccountPassword("123456")).toBeNull();
  });

  it("recognises refusals of the account creation", () => {
    expect(createAccountFailure({ reason: "usernameTaken" })).toBe("usernameTaken");
    expect(createAccountFailure({ reason: "other" })).toBeNull();
    expect(createAccountFailure(null)).toBeNull();
  });

  it("trims the note and caps it at the limit", () => {
    expect(normalizeNote("  привет  ")).toBe("привет");
    expect(normalizeNote("x".repeat(ACCOUNT_NOTE_MAX + 50))).toHaveLength(ACCOUNT_NOTE_MAX);
  });
});
