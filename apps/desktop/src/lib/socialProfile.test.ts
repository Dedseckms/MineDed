import { describe, expect, it } from "vitest";

import {
  ACCENTS,
  MAX_BIO,
  MAX_DISPLAY_NAME,
  accentColor,
  accentOption,
  canEditProfile,
  displayNameOf,
  profilePatch,
  validateBio,
  validateDisplayName,
} from "./socialProfile";

const user = (patch: Partial<Parameters<typeof displayNameOf>[0]> = {}) => ({
  name: "Steve",
  displayName: null,
  ...patch,
});

describe("profile helpers", () => {
  it("falls back to the nickname when the display name is empty", () => {
    expect(displayNameOf(user())).toBe("Steve");
    expect(displayNameOf(user({ displayName: "  " }))).toBe("Steve");
    expect(displayNameOf(user({ displayName: "Steve the Miner" }))).toBe(
      "Steve the Miner",
    );
  });

  it("limits the display name to 24 characters and the bio to 120", () => {
    expect(MAX_DISPLAY_NAME).toBe(24);
    expect(validateDisplayName("a".repeat(24))).toBeNull();
    expect(validateDisplayName("a".repeat(25))).toBe(
      "social.profile.nameTooLong",
    );
    expect(validateDisplayName("   ")).toBeNull();
    expect(MAX_BIO).toBe(120);
    expect(validateBio("a".repeat(120))).toBeNull();
    expect(validateBio("a".repeat(121))).toBe("social.profile.bioTooLong");
  });

  it("sends empty strings to clear fields and keeps the accent id", () => {
    expect(
      profilePatch({ displayName: "  Alex  ", bio: "", accent: "amber" }),
    ).toEqual({ displayName: "Alex", bio: "", accent: "amber" });
    expect(
      profilePatch({ displayName: "", bio: " hi ", accent: null }),
    ).toEqual({ displayName: "", bio: "hi", accent: "" });
  });

  it("resolves accents from the fixed palette only", () => {
    expect(ACCENTS.length).toBeGreaterThanOrEqual(8);
    expect(accentOption("violet")?.swatch).toMatch(/^#[0-9a-f]{6}$/i);
    expect(accentOption("neon")).toBeNull();
    expect(accentOption(null)).toBeNull();
    expect(accentColor("violet", "dark")).not.toBeNull();
    expect(accentColor("violet", "light")).not.toBe(
      accentColor("violet", "dark"),
    );
  });

  it("lets only full server accounts edit a profile", () => {
    const base = {
      ...user(),
      uuid: "1".repeat(32),
      skinUrl: null,
      status: null,
      role: "user" as const,
      isFriend: false,
      canMessage: true,
      canInvite: true,
    };
    expect(canEditProfile(base)).toBe(true);
    expect(canEditProfile({ ...base, role: "guest", guest: true })).toBe(false);
    expect(canEditProfile({ ...base, guest: true })).toBe(false);
  });
});
