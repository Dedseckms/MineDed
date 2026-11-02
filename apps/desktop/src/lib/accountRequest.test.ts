// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";

import {
  ACCOUNT_NOTE_MAX,
  hasRequestedAccount,
  markAccountRequested,
  normalizeNote,
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

  it("trims the note and caps it at the limit", () => {
    expect(normalizeNote("  привет  ")).toBe("привет");
    expect(normalizeNote("x".repeat(ACCOUNT_NOTE_MAX + 50))).toHaveLength(ACCOUNT_NOTE_MAX);
  });
});
