import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import { Icon } from "../../components/ui/Icon";

/** The hub keeps a presence for 120 s and ticks every 60 s; refresh once a minute. */
export const PRESENCE_INTERVAL_MS = 60_000;

/**
 * Public launcher counter (`social_presence`): any account kind counts, only
 * the number is public. Renders nothing until the first answer.
 */
export function OnlineBadge({ enabled = true }: { enabled?: boolean }) {
  const { t } = useTranslation();
  const presence = useQuery({
    queryKey: ["social", "presence"],
    queryFn: async () => unwrap(await api.socialPresence()),
    enabled,
    refetchInterval: PRESENCE_INTERVAL_MS,
    staleTime: PRESENCE_INTERVAL_MS,
    retry: false,
  });
  const online = presence.data?.online;
  if (online === undefined) return null;
  const label = t("social.online", { n: online });
  return (
    <span
      role="status"
      aria-label={label}
      title={label}
      className="inline-flex items-center gap-1 rounded-md bg-surface-2 px-1.5 py-0.5 text-xs font-medium text-muted"
    >
      <Icon name="users" className="size-3.5" />
      <span aria-hidden>{online}</span>
    </span>
  );
}
