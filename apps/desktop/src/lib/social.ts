import type {
  SocialAudience,
  SocialPrivacy,
  SocialRole,
  SocialUser,
} from "../bindings";

export const MAX_CHAT_FILE = 10 * 1024 * 1024;
export function canModerate(role: SocialRole): boolean {
  return role === "moderator" || role === "admin";
}
export function privacyOptions(field: keyof SocialPrivacy): SocialAudience[] {
  if (field === "whoCanInvite") return ["everyone", "friends"];
  if (field === "whoCanAddFriend") return ["everyone", "nobody"];
  return ["everyone", "friends", "nobody"];
}
export function canContact(user: SocialUser, blocked: boolean): boolean {
  return user.canMessage && !blocked;
}
