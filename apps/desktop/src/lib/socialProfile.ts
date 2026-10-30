import type { ProfileUpdate, SocialUser } from "../bindings";
import type { ResolvedTheme } from "../hooks/useAppearance";

export const MAX_DISPLAY_NAME = 24;
export const MAX_BIO = 120;

export interface AccentOption {
  /** Stored on the hub; ids are permanent, so never rename or reorder. */
  id: string;
  /** Chip colour for the swatches (same in both themes). */
  swatch: string;
  /** Readable name colour on a light background. */
  light: string;
  /** Readable name colour on a dark background. */
  dark: string;
}

/** Fixed accent palette: the hub rejects ids outside this list. */
export const ACCENTS: readonly AccentOption[] = [
  { id: "violet", swatch: "#7c5cff", light: "#5b3fd6", dark: "#b3a1ff" },
  { id: "blue", swatch: "#3b82f6", light: "#1d4ed8", dark: "#93c5fd" },
  { id: "cyan", swatch: "#06b6d4", light: "#0e7490", dark: "#67e8f9" },
  { id: "teal", swatch: "#14b8a6", light: "#0f766e", dark: "#5eead4" },
  { id: "green", swatch: "#22c55e", light: "#15803d", dark: "#86efac" },
  { id: "lime", swatch: "#84cc16", light: "#4d7c0f", dark: "#bef264" },
  { id: "amber", swatch: "#f59e0b", light: "#b45309", dark: "#fcd34d" },
  { id: "orange", swatch: "#f97316", light: "#c2410c", dark: "#fdba74" },
  { id: "rose", swatch: "#f43f5e", light: "#be123c", dark: "#fda4af" },
  { id: "pink", swatch: "#ec4899", light: "#be185d", dark: "#f9a8d4" },
];

export function accentOption(id: string | null | undefined): AccentOption | null {
  if (!id) return null;
  return ACCENTS.find((accent) => accent.id === id) ?? null;
}

/** Name colour of an accent, or `null` for the stable per-player hue. */
export function accentColor(
  id: string | null | undefined,
  theme: ResolvedTheme,
): string | null {
  const option = accentOption(id);
  return option ? option[theme] : null;
}

/** Name shown in chat: the chosen display name, else the profile nickname. */
export function displayNameOf(
  user: Pick<SocialUser, "name" | "displayName">,
): string {
  return user.displayName?.trim() || user.name;
}

/** Only full server accounts edit profiles; guests are read-only. */
export function canEditProfile(user: SocialUser): boolean {
  return user.role !== "guest" && user.guest !== true;
}

/** i18n key of the display-name error, or `null` when the value is valid. */
export function validateDisplayName(value: string): string | null {
  return value.trim().length > MAX_DISPLAY_NAME
    ? "social.profile.nameTooLong"
    : null;
}

/** i18n key of the bio error, or `null` when the value is valid. */
export function validateBio(value: string): string | null {
  return value.length > MAX_BIO ? "social.profile.bioTooLong" : null;
}

/** One save: `""` clears a field on the hub, so empty means "fall back". */
export function profilePatch(draft: {
  displayName: string;
  bio: string;
  accent: string | null;
}): ProfileUpdate {
  return {
    displayName: draft.displayName.trim(),
    bio: draft.bio.trim(),
    accent: draft.accent ?? "",
  };
}
