import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { AccountRequest } from "../../../bindings";
import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { CopyButton } from "../../../components/ui/CopyButton";
import { Icon } from "../../../components/ui/Icon";
import { ErrorMessage } from "../../../components/ui/feedback";
import { useToastStore } from "../../../stores/toastStore";
import { AdminSplit, ListRow, useSocialAction } from "./shared";

/** Guest requests for a real account: review the note, issue a Drasl invite. */
export function RequestsTab() {
  const { t, i18n } = useTranslation();
  const [selected, setSelected] = useState<string | null>(null);
  /** Links issued in this session, by request id: the list query may lag behind. */
  const [issued, setIssued] = useState<Record<string, string>>({});
  const requests = useQuery({
    queryKey: ["social", "accountRequests"],
    queryFn: async () => unwrap(await api.socialAccountRequests()),
    refetchInterval: 30000,
  });
  const action = useSocialAction();
  const list = requests.data ?? [];
  const current = list.find((r) => r.id === selected) ?? list[0] ?? null;
  const when = (at: string) => new Date(at).toLocaleString(i18n.language);

  const invite = (request: AccountRequest) =>
    action.mutate(async () => {
      const link = unwrap(await api.socialIssueInvite(request.id));
      setIssued((prev) => ({ ...prev, [request.id]: link }));
      useToastStore.getState().push({
        tone: "info",
        messageKey: "social.admin.requests.issuedToast",
        dedupeKey: "accountInvite",
      });
    });

  const link = current ? (issued[current.id] ?? null) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto lg:overflow-hidden">
      <ErrorMessage error={action.error ?? requests.error} className="shrink-0" />
      <AdminSplit
        list={
          <div className="min-h-0 flex-1 overflow-y-auto">
            {list.length === 0 && !requests.isFetching ? (
              <p className="px-3 py-4 text-sm text-muted">
                {t("social.admin.requests.empty")}
              </p>
            ) : null}
            {list.map((request) => (
              <ListRow
                key={request.id}
                active={current?.id === request.id}
                onClick={() => setSelected(request.id)}
              >
                <span className="flex w-full items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {request.name}
                  </span>
                  {issued[request.id] ? (
                    <span className="text-accent" aria-hidden>
                      <Icon name="check" />
                    </span>
                  ) : null}
                </span>
                <span className="truncate text-xs text-muted">
                  {when(request.createdAt)}
                </span>
                {request.note ? (
                  <span className="line-clamp-2 text-xs text-muted">
                    {request.note}
                  </span>
                ) : null}
              </ListRow>
            ))}
          </div>
        }
        details={
          current ? (
            <div className="flex flex-col gap-4">
              <header className="flex items-center gap-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-lg font-semibold">{current.name}</p>
                  <p className="text-xs text-muted">{when(current.createdAt)}</p>
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  loading={action.isPending && !link}
                  icon={<Icon name="link" />}
                  onClick={() => invite(current)}
                >
                  {t("social.admin.requests.invite")}
                </Button>
              </header>
              <section className="space-y-1">
                <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  {t("social.admin.requests.note")}
                </p>
                <p className="text-sm break-words whitespace-pre-wrap">
                  {current.note || t("social.admin.requests.noNote")}
                </p>
              </section>
              {link ? (
                <section className="space-y-2 rounded-lg border border-border bg-surface-2 p-3">
                  <p className="text-sm font-medium">
                    {t("social.admin.requests.inviteLink")}
                  </p>
                  <p className="font-mono text-xs break-all select-all">{link}</p>
                  <CopyButton value={link} label={t("social.admin.requests.copy")} />
                  <p className="text-xs text-muted">
                    {t("social.admin.requests.inviteHint")}
                  </p>
                </section>
              ) : null}
            </div>
          ) : (
            <p className="text-sm text-muted">
              {t("social.admin.requests.select")}
            </p>
          )
        }
      />
    </div>
  );
}
