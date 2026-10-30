import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SocialUser } from "../../bindings";
import { api, unwrap } from "../../api/client";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/form";
import { ErrorMessage } from "../../components/ui/feedback";
import { VirtualList } from "../../components/ui/VirtualList";
import { canModerate } from "../../lib/social";

export function SocialAdmin({ self }: { self: SocialUser }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [target, setTarget] = useState<SocialUser | null>(null);
  const allowed = canModerate(self.role);
  const reports = useQuery({
    queryKey: ["social", "reports"],
    queryFn: async () => unwrap(await api.socialReports()),
    enabled: allowed,
  });
  const server = useQuery({
    queryKey: ["social", "server"],
    queryFn: async () => unwrap(await api.socialServerState()),
    enabled: self.role === "admin",
  });
  const users = useQuery({
    queryKey: ["social", "adminSearch", query],
    queryFn: async () => unwrap(await api.socialSearchUsers(query)),
    enabled: allowed && !!query,
  });
  const action = useMutation({
    mutationFn: (run: () => Promise<unknown>) => run(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["social"] }),
  });
  const run = (fn: () => Promise<unknown>) => action.mutate(fn);
  if (!allowed) return null;
  return (
    <section className="space-y-4">
      <ErrorMessage
        error={action.error ?? reports.error ?? server.error ?? users.error}
      />
      <h3 className="font-semibold">{t("social.reports")}</h3>
      {reports.data?.length === 0 ? (
        <p className="text-muted">{t("social.noReports")}</p>
      ) : null}
      <VirtualList
        items={reports.data ?? []}
        height={320}
        rowHeight={152}
        getKey={(r) => r.id}
        ariaLabel={t("social.reports")}
        renderRow={(r) => (
          <article className="m-1 h-36 overflow-auto rounded-lg border border-border bg-panel p-3">
            <p className="font-medium">{r.message.author.name}</p>
            <p className="break-words">
              {r.message.content.type === "text"
                ? r.message.content.text
                : t("social.attachment")}
            </p>
            <p className="text-sm text-muted">{r.reason}</p>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialModerateDelete(r.message.id));
                  })
                }
              >
                {t("social.moderateDelete")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialResolveReport(r.id));
                  })
                }
              >
                {t("social.resolve")}
              </Button>
              <Button size="sm" onClick={() => setTarget(r.message.author)}>
                {t("social.moderateUser")}
              </Button>
            </div>
          </article>
        )}
      />
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(search.trim());
        }}
      >
        <TextInput
          aria-label={t("social.search")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <Button type="submit">{t("common.search")}</Button>
      </form>
      <VirtualList
        items={users.data ?? []}
        height={200}
        rowHeight={48}
        getKey={(u) => u.uuid}
        renderRow={(u) => (
          <button
            className="w-full p-2 text-left hover:bg-surface-2"
            onClick={() => setTarget(u)}
          >
            {u.name} · {t(`social.role.${u.role}`)}
          </button>
        )}
      />
      {target ? (
        <div className="space-y-2 rounded-lg border border-border bg-panel p-4">
          <h3>{target.name}</h3>
          <div className="flex flex-wrap gap-2">
            {[3600, 86400].map((seconds) => (
              <Button
                key={seconds}
                size="sm"
                disabled={action.isPending}
                onClick={() =>
                  run(async () => {
                    unwrap(
                      await api.socialModerateUser(
                        target.uuid,
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
              onClick={() =>
                run(async () => {
                  unwrap(
                    await api.socialModerateUser(target.uuid, "unmute", null),
                  );
                })
              }
            >
              {t("social.unmute")}
            </Button>
            {self.role === "admin" ? (
              <>
                <Button
                  size="sm"
                  onClick={() =>
                    run(async () => {
                      unwrap(
                        await api.socialModerateUser(target.uuid, "ban", null),
                      );
                    })
                  }
                >
                  {t("social.ban")}
                </Button>
                <Button
                  size="sm"
                  onClick={() =>
                    run(async () => {
                      unwrap(
                        await api.socialModerateUser(
                          target.uuid,
                          "unban",
                          null,
                        ),
                      );
                    })
                  }
                >
                  {t("social.unban")}
                </Button>
              </>
            ) : null}
          </div>
          {self.role === "admin" ? (
            <div className="flex gap-2">
              {(["moderator", "user"] as const).map((role) => (
                <Button
                  key={role}
                  size="sm"
                  variant="secondary"
                  onClick={() =>
                    run(async () => {
                      unwrap(await api.socialSetRole(target.uuid, role));
                    })
                  }
                >
                  {t(`social.setRole.${role}`)}
                </Button>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
      {self.role === "admin" && server.data ? (
        <div className="space-y-3 rounded-xl border border-border bg-panel p-4">
          <h3 className="font-semibold">{t("social.server")}</h3>
          <p>
            {t("social.disk", {
              used: (server.data.diskUsedBytes / 1024 ** 3).toFixed(1),
              total: (server.data.diskTotalBytes / 1024 ** 3).toFixed(1),
            })}
          </p>
          {server.data.attachmentsPaused ? (
            <p role="alert" className="text-warning">
              {t("social.diskFull")}
            </p>
          ) : null}
          <p>
            {t("social.backup", {
              date: server.data.lastBackup
                ? new Date(server.data.lastBackup).toLocaleString()
                : t("social.noBackup"),
            })}
          </p>
          {server.data.backupError ? (
            <p role="alert" className="text-warning">
              {t("social.backupError")}
            </p>
          ) : null}
          {server.data.migrationProgress !== null ? (
            <p role="status">
              {t("social.migrating", {
                percent: Math.round(server.data.migrationProgress * 100),
              })}
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            {(["newDisk", "objectStorage"] as const).map((target) => (
              <Button
                key={target}
                variant="secondary"
                disabled={
                  action.isPending || server.data?.migrationProgress !== null
                }
                onClick={() =>
                  run(async () => {
                    unwrap(await api.socialMigrateStorage(target));
                  })
                }
              >
                {t(`social.storage.${target}`)}
              </Button>
            ))}
          </div>
          <p className="text-xs text-muted">{t("social.storageHint")}</p>
        </div>
      ) : null}
    </section>
  );
}
