import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { ErrorMessage } from "../../../components/ui/feedback";

/** Action ids the hub writes; unknown ids are shown verbatim. */
const ACTIONS = new Set([
  "role",
  "mute",
  "unmute",
  "ban",
  "unban",
  "delete",
  "resolve",
  "profileReset",
  "avatarReset",
  "migrate",
]);

/** When / who / what / over whom. */
export function JournalTab() {
  const { t, i18n } = useTranslation();
  const audit = useQuery({
    queryKey: ["social", "adminAudit"],
    queryFn: async () => unwrap(await api.socialAdminAudit()),
    refetchInterval: 60000,
  });
  const actionLabel = (action: string) =>
    ACTIONS.has(action)
      ? t(`social.admin.action.${action}`)
      : action;
  const entries = audit.data ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <ErrorMessage
          error={audit.error}
          onRetry={() => void audit.refetch()}
          className="min-w-0 flex-1"
        />
        <Button
          size="sm"
          variant="ghost"
          loading={audit.isFetching}
          onClick={() => void audit.refetch()}
        >
          {t("social.admin.refresh")}
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto rounded-xl border border-border bg-panel">
        {entries.length === 0 && !audit.isPending ? (
          <p className="p-4 text-sm text-muted">
            {t("social.admin.journal.empty")}
          </p>
        ) : null}
        {entries.length > 0 ? (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-surface-2 text-left text-xs text-muted">
              <tr>
                <th className="px-3 py-2 font-medium">
                  {t("social.admin.journal.when")}
                </th>
                <th className="px-3 py-2 font-medium">
                  {t("social.admin.journal.who")}
                </th>
                <th className="px-3 py-2 font-medium">
                  {t("social.admin.journal.what")}
                </th>
                <th className="px-3 py-2 font-medium">
                  {t("social.admin.journal.whom")}
                </th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.id} className="border-t border-border">
                  <td className="px-3 py-2 whitespace-nowrap text-muted">
                    <time dateTime={entry.at}>
                      {new Date(entry.at).toLocaleString(i18n.language)}
                    </time>
                  </td>
                  <td className="px-3 py-2 break-words">{entry.actor}</td>
                  <td className="px-3 py-2 break-words" title={entry.note ?? undefined}>
                    {actionLabel(entry.action)}
                  </td>
                  <td className="px-3 py-2 break-words">
                    {entry.target ?? t("social.admin.journal.nobody")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : null}
      </div>
    </div>
  );
}
