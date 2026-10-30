import type { SocialUser } from "../../bindings";
import { Icon, type IconName } from "../../components/ui/Icon";
import { useResolvedTheme } from "../../hooks/useAppearance";
import { initials, userColor } from "../../lib/social";

const SIZES = {
  sm: "size-8 text-[11px]",
  md: "size-10 text-sm",
} as const;

/** Round placeholder avatar: player initials in a stable colour (no images, no reflow). */
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
  return (
    <span
      aria-hidden
      className={`grid shrink-0 place-items-center rounded-full bg-surface-2 font-semibold ring-1 ring-border ${SIZES[size]}`}
      style={user ? { color: userColor(user.uuid, theme) } : undefined}
    >
      {user ? (
        initials(user.name)
      ) : (
        <Icon name={icon} className="size-4 text-muted" />
      )}
    </span>
  );
}
