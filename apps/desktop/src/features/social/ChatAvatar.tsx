import type { SocialUser } from "../../bindings";
import { Icon, type IconName } from "../../components/ui/Icon";
import { useAvatarPreview } from "../../hooks/useAvatarPreview";
import { useResolvedTheme } from "../../hooks/useAppearance";
import { initials, userColor } from "../../lib/social";
import { accentColor, displayNameOf } from "../../lib/socialProfile";

const SIZES = {
  sm: "size-8 text-[11px]",
  md: "size-10 text-sm",
  lg: "size-16 text-xl",
} as const;

/**
 * Round avatar: the uploaded picture when the hub has one, otherwise player
 * initials in the profile accent colour (or the stable per-player hue).
 * Both variants share the box size, so the list never reflows.
 */
export function ChatAvatar({
  user,
  icon = "users",
  size = "md",
}: {
  user: SocialUser | null;
  icon?: IconName;
  size?: keyof typeof SIZES;
}) {
  const theme = useResolvedTheme();
  const accent = accentColor(user?.accent, theme);
  const picture = useAvatarPreview(user);
  return (
    <span
      aria-hidden
      className={`grid shrink-0 place-items-center overflow-hidden rounded-full bg-surface-2 font-semibold ring-1 ring-border ${SIZES[size]}`}
      style={
        user
          ? {
              color: accent ?? userColor(user.uuid, theme),
              backgroundColor: accent
                ? `color-mix(in srgb, ${accent} 16%, transparent)`
                : undefined,
            }
          : undefined
      }
    >
      {picture ? (
        <img
          src={picture}
          alt=""
          draggable={false}
          className="size-full object-cover"
        />
      ) : user ? (
        initials(displayNameOf(user))
      ) : (
        <Icon name={icon} className="size-4 text-muted" />
      )}
    </span>
  );
}
