import { describe, expect, it } from "vitest";
import type { Account, SocialRole } from "../bindings";
import { createMockBus } from "./bus";
import { createSocialMocks, SOCIAL_PEER } from "./social";
const account: Account = {
  id: "own",
  uuid: "11111111-1111-1111-1111-111111111111",
  name: "Player",
  kind: "yggdrasil",
  server: "https://auth.example",
  skinUrl: null,
  valid: true,
  isDefault: true,
};
function mock(role: SocialRole = "user") {
  return createSocialMocks(
    { bus: createMockBus(), findAccount: () => account, runTask: () => "task" },
    role,
  );
}
describe("social service mock", () => {
  it("denies private data before connection", async () => {
    const api = mock();
    expect((await api.socialChannels!()).status).toBe("error");
    expect((await api.socialPrivacy!()).status).toBe("error");
  });
  it("accepts incoming friendship and prevents blocked direct chats", async () => {
    const api = mock();
    await api.socialConnect!("own");
    expect(await api.socialAddFriend!(SOCIAL_PEER.uuid)).toMatchObject({
      status: "ok",
      data: { state: "friends" },
    });
    await api.socialSetBlocked!(SOCIAL_PEER.uuid, true);
    expect((await api.socialOpenDirect!(SOCIAL_PEER.uuid)).status).toBe(
      "error",
    );
    await api.socialSetBlocked!(SOCIAL_PEER.uuid, false);
    expect((await api.socialOpenDirect!(SOCIAL_PEER.uuid)).status).toBe("ok");
  });
  it("persists privacy and denies user access to administration", async () => {
    const api = mock();
    await api.socialConnect!("own");
    const privacy = {
      whoCanMessage: "nobody",
      whoCanInvite: "friends",
      whoSeesStatus: "friends",
      whoCanAddFriend: "nobody",
    } as const;
    await api.socialSetPrivacy!(privacy);
    expect(await api.socialPrivacy!()).toMatchObject({
      status: "ok",
      data: privacy,
    });
    expect((await api.socialReports!()).status).toBe("error");
    expect(
      (await api.socialSetRole!(SOCIAL_PEER.uuid, "moderator")).status,
    ).toBe("error");
  });
  it("bounds moderator authority to a day and keeps server/roles admin-only", async () => {
    const api = mock("moderator");
    await api.socialConnect!("own");
    expect(
      (await api.socialModerateUser!(SOCIAL_PEER.uuid, "mute", 86400)).status,
    ).toBe("ok");
    expect(
      (await api.socialModerateUser!(SOCIAL_PEER.uuid, "mute", 86401)).status,
    ).toBe("error");
    expect(
      (await api.socialModerateUser!(SOCIAL_PEER.uuid, "ban", null)).status,
    ).toBe("error");
    expect((await api.socialServerState!()).status).toBe("error");
    expect(
      (await api.socialSetRole!(SOCIAL_PEER.uuid, "moderator")).status,
    ).toBe("error");
  });
  it("allows admin server tools", async () => {
    const api = mock("admin");
    await api.socialConnect!("own");
    expect((await api.socialServerState!()).status).toBe("ok");
    expect(
      (await api.socialSetRole!(SOCIAL_PEER.uuid, "moderator")).status,
    ).toBe("ok");
  });
  it("edits the own profile and rejects values outside the limits", async () => {
    const api = mock();
    await api.socialConnect!("own");
    expect(
      await api.socialProfileUpdate!({
        displayName: "Steve the Miner",
        bio: "hi",
        accent: "amber",
      }),
    ).toMatchObject({
      status: "ok",
      data: { displayName: "Steve the Miner", bio: "hi", accent: "amber" },
    });
    expect(
      (await api.socialProfileUpdate!({ displayName: "a".repeat(25) })).status,
    ).toBe("error");
    expect(
      (await api.socialProfileUpdate!({ bio: "a".repeat(121) })).status,
    ).toBe("error");
    expect(
      (await api.socialProfileUpdate!({ accent: "neon" })).status,
    ).toBe("error");
    expect(
      (await api.socialProfileUpdate!({ displayName: "", accent: "" })).status,
    ).toBe("ok");
  });
  it("keeps presence and the audit journal admin-only", async () => {
    const user = mock();
    await user.socialConnect!("own");
    expect((await user.socialAdminPresence!()).status).toBe("error");
    expect((await user.socialAdminAudit!()).status).toBe("error");
    const api = mock("admin");
    await api.socialConnect!("own");
    const presence = await api.socialAdminPresence!();
    expect(presence.status === "ok" && presence.data.clients.length > 0).toBe(
      true,
    );
    await api.socialModerateUser!(SOCIAL_PEER.uuid, "mute", 3600);
    const audit = await api.socialAdminAudit!();
    if (audit.status !== "ok") throw new Error("expected ok");
    expect(audit.data[0]).toMatchObject({
      action: "mute",
      target: SOCIAL_PEER.name,
    });
  });
});
