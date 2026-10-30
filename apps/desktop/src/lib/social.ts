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

/** Stable per-player hue, so names and avatars keep their colour across screens. */
const CHAT_HUES = [8, 28, 48, 88, 148, 178, 205, 232, 268, 310] as const;

export function userHue(seed: string): number {
  let hash = 0;
  for (let i = 0; i < seed.length; i++)
    hash = (hash * 31 + seed.charCodeAt(i)) % 100_000;
  return CHAT_HUES[hash % CHAT_HUES.length] ?? 205;
}

/** Text colour for a player name on a bubble; keeps AA contrast in both themes. */
export function userColor(seed: string, theme: "light" | "dark"): string {
  const hue = userHue(seed);
  return theme === "light"
    ? `hsl(${hue} 72% 30%)`
    : `hsl(${hue} 78% 72%)`;
}

/** One or two letters for the avatar placeholder. */
export function initials(name: string): string {
  const parts = name.trim().split(/[\s_-]+/).filter(Boolean);
  const first = parts[0]?.slice(0, 1) ?? "?";
  const second = parts.length > 1 ? (parts[1]?.slice(0, 1) ?? "") : "";
  return `${first}${second}`.toLocaleUpperCase();
}
