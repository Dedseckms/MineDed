import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { AccountRequest, SocialAppeal } from "../../../bindings";
import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { Icon } from "../../../components/ui/Icon";
import { ErrorMessage } from "../../../components/ui/feedback";
import { createAccountFailure } from "../../../lib/accountRequest";
import { socialAdminCreateAccount } from "../../../lib/socialAdmin";
import { useToastStore } from "../../../stores/toastStore";
import { AdminSplit, ListRow, useSocialAction } from "./shared";

/** Guest requests for a real account: review the login and note, create the account. */
export function RequestsTab() {
  const { t, i18n } = useTranslation();
  const [selected, setSelected] = useState<string | null>(null);
  /** Requests handled in this session: the list query may lag behind. */
  const [handled, setHandled] = useState<Record<string, "created" | "rejected">>({});
  /** Per-request failure of the last "create account" action. */
  const [failed, setFailed] = useState<{ id: string; error: unknown } | null>(null);
  const requests = useQuery({
    queryKey: ["social", "accountRequests"],
    queryFn: async () => unwrap(await api.socialAccountRequests()),
    refetchInterval: 30000,
  });
  const action = useSocialAction();
  const list = (requests.data ?? []).filter((r) => !handled[r.id]);
  const current = list.find((r) => r.id === selected) ?? list[0] ?? null;
  const when = (at: string) => new Date(at).toLocaleString(i18n.language);

  // Ban appeals of banned players: accept lifts the ban, dismiss closes it.
  const appeals = useQuery({
    queryKey: ["social", "appeals"],
    queryFn: async () => unwrap(await api.socialAppeals()),
    refetchInterval: 30000,
  });
  const resolveAppeal = (appeal: SocialAppeal, accept: boolean) =>
    action.mutate(async () => {
      unwrap(await api.socialResolveAppeal(appeal.uuid, accept));
    });

  const create = (request: AccountRequest) =>
    action.mutate(
      async () => {
        setFailed(null);
        unwrap(await socialAdminCreateAccount(request.id));
      },
      {
        onSuccess: () => {
          setHandled((prev) => ({ ...prev, [request.id]: "created" }));
          setSelected(null);
          useToastStore.getState().push({
            tone: "info",
            messageKey: "social.admin.requests.createdToast",
            dedupeKey: "accountCreated",
          });
        },
        onError: (error) => setFailed({ id: request.id, error }),
      },
    );

  const reject = (request: AccountRequest) =>
    action.mutate(
      async () => {
        setFailed(null);
        unwrap(await api.socialRejectAccount(request.id));
      },
      {
        onSuccess: () => {
          setHandled((prev) => ({ ...prev, [request.id]: "rejected" }));
          setSelected(null);
          useToastStore.getState().push({
            tone: "info",
            messageKey: "social.admin.requests.rejectedToast",
            dedupeKey: "accountRejected",
          });
        },
        onError: (error) => setFailed({ id: request.id, error }),
      },
    );

  const inlineFailure = (id: string): string | null => {
    if (failed?.id !== id) return null;
    const failure = createAccountFailure(
      (failed.error as { error?: { details?: Record<string, string> | null } } | null)?.error
        ?.details,
    );
    return failure ? t(`errors.reason.${failure}`) : null;
  };
  const failure = current ? inlineFailure(current.id) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto lg:overflow-hidden">
      <ErrorMessage error={requests.error} className="shrink-0" />
      {(appeals.data ?? []).length > 0 ? (
        <section className="shrink-0 rounded-lg border border-warning/40 bg-warning/5 p-3">
          <p className="mb-2 text-sm font-medium">
            {t("social.admin.appeals.title", { count: appeals.data?.length ?? 0 })}
          </p>
          <ul className="flex flex-col gap-2">
            {(appeals.data ?? []).map((appeal) => (
              <li key={appeal.uuid} className="flex flex-wrap items-center gap-2">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{appeal.name}</span>
                  <span className="block text-xs break-words text-muted">{appeal.text}</span>
                </span>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={action.isPending}
                  onClick={() => resolveAppeal(appeal, true)}
                >
                  {t("social.admin.appeals.accept")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={action.isPending}
                  onClick={() => resolveAppeal(appeal, false)}
                >
                  {t("social.admin.appeals.dismiss")}
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
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

                </span>
                <span className="truncate text-xs text-muted">
                  {when(request.createdAt)}
                </span>
                {request.note ? (
                  <span className="line-clamp-2 text-xs text-muted">
                    {request.note}
                  </span>
                ) : null}
                {inlineFailure(request.id) ? (
                  <span className="text-xs text-danger">{inlineFailure(request.id)}</span>
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
                  <p className="text-xs text-muted">
                    {current.login ? `@${current.login} · ` : ""}
                    {when(current.createdAt)}
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  loading={action.isPending}
                  icon={<Icon name="user" />}
                  onClick={() => create(current)}
                >
                  {t("social.admin.requests.create")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  className="text-danger"
                  icon={<Icon name="x" />}
                  onClick={() => reject(current)}
                >
                  {t("social.admin.requests.reject")}
                </Button>
              </header>
              {failure ? (
                <p role="alert" className="text-sm text-danger">
                  {failure}
                </p>
              ) : null}
              <section className="space-y-1">
                <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  {t("social.admin.requests.note")}
                </p>
                <p className="text-sm break-words whitespace-pre-wrap">
                  {current.note || t("social.admin.requests.noNote")}
                </p>
              </section>
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
