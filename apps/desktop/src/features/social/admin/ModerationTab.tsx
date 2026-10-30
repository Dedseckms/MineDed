import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { SocialMessage, SocialReport } from "../../../bindings";
import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { ErrorMessage, Badge } from "../../../components/ui/feedback";
import { Segmented } from "../../../components/ui/form";
import { displayNameOf } from "../../../lib/socialProfile";
import { useAdminStore } from "../../../stores/adminStore";
import { AdminSplit, ListRow, useSocialAction } from "./shared";

type ReportFilter = "open" | "resolved";

const MUTE_OPTIONS = [3600, 86400] as const;

function bodyOf(message: SocialMessage, fallback: string): string {
  return message.content.type === "text" ? message.content.text : fallback;
}

/** Reports: open ones from the hub, resolved ones from this session. */
export function ModerationTab() {
  const { t, i18n } = useTranslation();
  const resolved = useAdminStore((s) => s.resolved);
  const [filter, setFilter] = useState<ReportFilter>("open");
  const [selected, setSelected] = useState<string | null>(null);
  const reports = useQuery({
    queryKey: ["social", "reports"],
    queryFn: async () => unwrap(await api.socialReports()),
    refetchInterval: 30000,
  });
  const action = useSocialAction();
  const list: SocialReport[] =
    filter === "open" ? (reports.data ?? []) : resolved;
  const current = list.find((r) => r.id === selected) ?? list[0] ?? null;
  const when = (at: string) => new Date(at).toLocaleString(i18n.language);

  const resolve = (report: SocialReport) =>
    action.mutate(async () => {
      unwrap(await api.socialResolveReport(report.id));
      useAdminStore.getState().addResolved(report);
    });

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <ErrorMessage error={action.error ?? reports.error} />
      <AdminSplit
        list={
          <>
            <div className="shrink-0 border-b border-border p-2">
              <Segmented
                value={filter}
                ariaLabel={t("social.admin.reportFilter")}
                options={[
                  {
                    value: "open",
                    label: t("social.admin.filter.open"),
                    badge: <Badge tone="warning">{reports.data?.length ?? 0}</Badge>,
                  },
                  {
                    value: "resolved",
                    label: t("social.admin.filter.resolved"),
                    badge: <Badge>{resolved.length}</Badge>,
                  },
                ]}
                onChange={setFilter}
              />
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {list.length === 0 ? (
                <p className="px-3 py-4 text-sm text-muted">
                  {t(
                    filter === "open"
                      ? "social.noReports"
                      : "social.admin.noResolved",
                  )}
                </p>
              ) : null}
              {list.map((report) => (
                <ListRow
                  key={report.id}
                  active={current?.id === report.id}
                  onClick={() => setSelected(report.id)}
                >
                  <span className="flex w-full items-baseline gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {displayNameOf(report.message.author)}
                    </span>
                    <time
                      className="shrink-0 text-[11px] text-muted"
                      dateTime={report.createdAt}
                    >
                      {when(report.createdAt)}
                    </time>
                  </span>
                  <span className="line-clamp-2 text-xs break-words text-muted">
                    {bodyOf(report.message, t("social.attachment"))}
                  </span>
                </ListRow>
              ))}
            </div>
          </>
        }
        details={
          current ? (
            <div className="flex flex-col gap-3">
              <header className="flex flex-wrap items-center gap-2">
                <h4 className="text-base font-semibold break-words">
                  {displayNameOf(current.message.author)}
                </h4>
                {filter === "resolved" ? (
                  <Badge tone="accent">{t("social.admin.filter.resolved")}</Badge>
                ) : (
                  <Badge tone="warning">{t("social.admin.filter.open")}</Badge>
                )}
              </header>
              <p className="text-xs text-muted">
                {t("social.admin.reportReporter", {
                  name: displayNameOf(current.reporter),
                })}
                {" · "}
                <time dateTime={current.createdAt}>
                  {when(current.createdAt)}
                </time>
              </p>
              <div className="space-y-1 rounded-lg bg-surface-2 px-3 py-2">
                <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  {t("social.admin.reportMessage")}
                </p>
                <p className="text-sm break-words whitespace-pre-wrap">
                  {bodyOf(current.message, t("social.attachment"))}
                </p>
              </div>
              {current.reason ? (
                <div className="space-y-1">
                  <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                    {t("social.admin.reportReason")}
                  </p>
                  <p className="text-sm break-words">{current.reason}</p>
                </div>
              ) : null}
              {filter === "open" ? (
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={action.isPending}
                    onClick={() =>
                      action.mutate(async () => {
                        unwrap(
                          await api.socialModerateDelete(current.message.id),
                        );
                      })
                    }
                  >
                    {t("social.moderateDelete")}
                  </Button>
                  {MUTE_OPTIONS.map((seconds) => (
                    <Button
                      key={seconds}
                      size="sm"
                      variant="secondary"
                      disabled={action.isPending}
                      onClick={() =>
                        action.mutate(async () => {
                          unwrap(
                            await api.socialModerateUser(
                              current.message.author.uuid,
                              "mute",
                              seconds,
                            ),
                          );
                        })
                      }
                    >
                      {t("social.mute", { hours: seconds / 3600 })}
                    </Button>
                  ))}
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={action.isPending}
                    onClick={() =>
                      action.mutate(async () => {
                        unwrap(
                          await api.socialModerateUser(
                            current.message.author.uuid,
                            "ban",
                            null,
                          ),
                        );
                      })
                    }
                  >
                    {t("social.ban")}
                  </Button>
                  <Button
                    size="sm"
                    disabled={action.isPending}
                    onClick={() => resolve(current)}
                  >
                    {t("social.resolve")}
                  </Button>
                </div>
              ) : (
                <p className="text-sm text-muted">{t("social.admin.resolvedNote")}</p>
              )}
            </div>
          ) : (
            <p className="text-sm text-muted">{t("social.admin.selectReport")}</p>
          )
        }
      />
    </div>
  );
}
