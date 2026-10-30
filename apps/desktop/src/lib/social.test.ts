import { describe, expect, it } from "vitest";
import { canContact, canModerate, privacyOptions } from "./social";
import { SOCIAL_PEER } from "../mocks/social";

describe("social privacy and roles", () => {
  it("does not offer unsupported privacy choices", () => {
    expect(privacyOptions("whoCanInvite")).toEqual(["everyone", "friends"]);
    expect(privacyOptions("whoCanAddFriend")).toEqual(["everyone", "nobody"]);
    expect(privacyOptions("whoSeesStatus")).toContain("nobody");
  });
  it("requires both server permission and no local block to contact", () => {
    expect(canContact(SOCIAL_PEER, false)).toBe(true);
    expect(canContact(SOCIAL_PEER, true)).toBe(false);
    expect(canContact({ ...SOCIAL_PEER, canMessage: false }, false)).toBe(
      false,
    );
  });
  it("reserves moderation for assigned roles", () => {
    expect(canModerate("user")).toBe(false);
    expect(canModerate("moderator")).toBe(true);
    expect(canModerate("admin")).toBe(true);
  });
});
