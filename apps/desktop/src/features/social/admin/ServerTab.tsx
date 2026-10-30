import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../../api/client";
import { Button } from "../../../components/ui/Button";
import { Badge, ErrorMessage } from "../../../components/ui/feedback";
import { ProgressBar } from "../../../components/ui/ProgressBar";
import { AdminSplit, ListRow, useSocialAction } from "./shared";

const GiB = 1024 ** 3;

/** Presence, disk/backup health and storage migrations. */
export function ServerTab() {
  const { t, i18n } = useTranslation();
  const presence = useQuery({
    queryKey: ["social", "adminPresence"],
    queryFn: async () => unwrap(await api.socialAdminPresence()),
    refetchInterval: 30000,
  });
  const server = useQuery({
    queryKey: ["social", "server"],
    queryFn: async () => unwrap(await api.socialServerState()),
    refetchInterval: 30000,
  });
  const action = useSocialAction();
  const state = server.data;
  const used = state ? state.diskUsedBytes / state.diskTotalBytes : 0;
  const kindLabel = (kind: string) =>
    ["guest", "yggdrasil", "anonymous"].includes(kind)
      ? t(`social.admin.kind.${kind}`)
      : kind;
  const refresh = () => {
    void presence.refetch();
    void server.refetch();
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <ErrorMessage
          error={action.error ?? presence.error ?? server.error}
          className="min-w-0 flex-1"
        />
        <Button
          size="sm"
          variant="ghost"
          onClick={refresh}
          loading={presence.isFetching || server.isFetching}
        >
          {t("social.admin.refresh")}
        </Button>
      </div>
      <AdminSplit
        list={
          <>
            <header className="shrink-0 border-b border-border p-3">
              <p className="text-xs text-muted">{t("social.admin.online")}</p>
              <p className="text-3xl font-semibold">{presence.data?.online ?? "—"}</p>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <p className="px-3 pt-3 pb-1 text-[11px] font-semibold tracking-wide text-muted uppercase">
                {t("social.admin.clients")}
              </p>
              {(presence.data?.clients ?? []).length === 0 ? (
                <p className="px-3 py-2 text-sm text-muted">
                  {t("social.admin.noClients")}
                </p>
              ) : null}
              {(presence.data?.clients ?? []).map((client, index) => (
                <ListRow key={`${client.since}-${client.label}-${index}`}>
                  <span className="flex w-full items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm">
                      {client.label}
                    </span>
                    <Badge
                      tone={client.kind === "guest" ? "warning" : "neutral"}
                    >
                      {kindLabel(client.kind)}
                    </Badge>
                  </span>
                  <span className="text-[11px] text-muted">
                    {t("social.admin.since", {
                      time: new Date(client.since).toLocaleTimeString(
                        i18n.language,
                      ),
                    })}
                  </span>
                </ListRow>
              ))}
            </div>
          </>
        }
        details={
          state ? (
            <div className="flex flex-col gap-4">
              <section className="space-y-1">
                <p className="text-sm font-semibold">{t("social.server")}</p>
                <p className="text-sm">
                  {t("social.disk", {
                    used: (state.diskUsedBytes / GiB).toFixed(1),
                    total: (state.diskTotalBytes / GiB).toFixed(1),
                  })}
                </p>
                <ProgressBar
                  value={Math.min(1, used)}
                  tone={state.attachmentsPaused ? "danger" : "accent"}
                  label={t("social.server")}
                />
                {state.attachmentsPaused ? (
                  <p role="alert" className="text-sm text-warning">
                    {t("social.diskFull")}
                  </p>
                ) : null}
              </section>
              <section className="space-y-1">
                <p className="text-sm">
                  {t("social.backup", {
                    date: state.lastBackup
                      ? new Date(state.lastBackup).toLocaleString(i18n.language)
                      : t("social.noBackup"),
                  })}
                </p>
                {state.backupError ? (
                  <p role="alert" className="text-sm text-warning">
                    {t("social.backupError")}
                  </p>
                ) : null}
              </section>
              <section className="space-y-2">
                {state.migrationProgress !== null ? (
                  <>
                    <p role="status" className="text-sm">
                      {t("social.migrating", {
                        percent: Math.round(state.migrationProgress * 100),
                      })}
                    </p>
                    <ProgressBar value={state.migrationProgress} />
                  </>
                ) : null}
                <div className="flex flex-wrap gap-2">
                  {(["newDisk", "objectStorage"] as const).map((target) => (
                    <Button
                      key={target}
                      size="sm"
                      variant="secondary"
                      disabled={
                        action.isPending || state.migrationProgress !== null
                      }
                      onClick={() =>
                        action.mutate(async () => {
                          unwrap(await api.socialMigrateStorage(target));
                        })
                      }
                    >
                      {t(`social.storage.${target}`)}
                    </Button>
                  ))}
                </div>
                <p className="text-xs text-muted">{t("social.storageHint")}</p>
              </section>
            </div>
          ) : (
            <p className="text-sm text-muted">{t("common.loading")}</p>
          )
        }
      />
    </div>
  );
}
