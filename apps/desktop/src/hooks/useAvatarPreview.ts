import { useQuery } from "@tanstack/react-query";

import { api, unwrap } from "../api/client";
import type { SocialUser } from "../bindings";
import { avatarQueryKey, isImageDataUrl } from "../lib/social";

/**
 * Cached avatar preview of a player as a data URL, or `null` while it loads,
 * when the player has no avatar, or when the hub refuses the request.
 *
 * Keyed by the avatar id, so the command runs once per uploaded image and
 * never for players without one. Callers fall back to initials.
 */
export function useAvatarPreview(
  user: Pick<SocialUser, "uuid" | "avatar"> | null | undefined,
): string | null {
  const uuid = user?.uuid ?? "";
  const avatar = user?.avatar ?? null;
  const query = useQuery({
    queryKey: avatarQueryKey(uuid, avatar),
    queryFn: async () => unwrap(await api.socialAvatarPreview(uuid)),
    enabled: uuid !== "" && avatar !== null && avatar !== "",
    staleTime: Infinity,
    // A missing avatar is a normal state, not a transient failure.
    retry: false,
  });
  return isImageDataUrl(query.data) ? (query.data ?? null) : null;
}

/**
 * Head of the player's Minecraft skin as a data URL, for players without an
 * uploaded avatar. Keyed by the skin URL, so a skin change (a new URL on the
 * profile) fetches a fresh head and no stale one is ever shown.
 */
export function useSkinHead(
  user: Pick<SocialUser, "skinUrl"> | null | undefined,
): string | null {
  const skin = user?.skinUrl ?? null;
  const query = useQuery({
    queryKey: ["skin", "head", skin ?? ""],
    queryFn: async () => unwrap(await api.skinHead(skin ?? "")),
    enabled: !!skin,
    staleTime: 30 * 60_000,
    // A player without a reachable skin keeps the initials placeholder.
    retry: false,
  });
  return isImageDataUrl(query.data) ? (query.data ?? null) : null;
}
