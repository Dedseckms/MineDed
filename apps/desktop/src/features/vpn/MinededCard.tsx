import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { VpnAccess, VpnState } from "../../bindings";
import { useAccounts, useMinededAccess, useMinededConnect, useSetVpnMode } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ProgressBar } from "../../components/ui/ProgressBar";
import { Badge, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { Field, Select } from "../../components/ui/form";
import { toAppError } from "../../lib/errors";
import { minededAccounts, minededUsage, toGb } from "../../lib/vpn";
import { useNavStore } from "../../stores/navStore";

function OpenAccounts() {
  const { t } = useTranslation();
  const go = useNavStore((s) => s.go);
  return (
    <Button size="sm" variant="ghost" icon={<Icon name="user" />} onClick={() => go("accounts")}>
      {t("vpn.mineded.openAccounts")}
    </Button>
  );
}

function Usage({ access }: { access: VpnAccess }) {
  const { t, i18n } = useTranslation();
  const u = minededUsage(access);
  const nf = new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 1 });
  const date = access.resetAt ? new Date(access.resetAt).toLocaleDateString(i18n.language) : null;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-3 text-sm">
        <span className="text-muted">{t("vpn.mineded.usage")}</span>
        <span className="tabular-nums">
          {t("vpn.mineded.ofTotal", { used: nf.format(toGb(access.usedBytes)), total: nf.format(toGb(access.quotaBytes)) })}
        </span>
      </div>
      <ProgressBar value={u.used} tone={u.exhausted ? "danger" : "accent"} label={t("vpn.mineded.usage")} />
      {u.exhausted ? (
        <p className="text-xs text-danger">
          {date ? t("vpn.mineded.exhausted", { date }) : t("vpn.mineded.exhaustedNoDate")}
        </p>
      ) : (
        <p className="text-xs text-muted">
          {date
            ? t("vpn.mineded.left", { left: nf.format(toGb(u.leftBytes)), date })
            : t("vpn.mineded.leftNoDate", { left: nf.format(toGb(u.leftBytes)) })}
        </p>
      )}
    </div>
  );
}

/** Personal MineDed VPN for own-server accounts: one-click connect and quota usage. */
export function MinededCard({ state }: { state: VpnState | undefined }) {
  const { t } = useTranslation();
  const accounts = useAccounts();
  const candidates = minededAccounts(accounts.data ?? []);
  const [picked, setPicked] = useState<string | null>(null);
  const account = candidates.find((a) => a.id === picked) ?? candidates[0] ?? null;
  const access = useMinededAccess(account?.id ?? null);
  const connect = useMinededConnect();
  const setMode = useSetVpnMode();

  const data = access.data ?? null;
  const exhausted = data ? minededUsage(data).exhausted : false;
  const running = !!state?.running && !!data;
  const error = connect.error ?? access.error ?? setMode.error;
  const authExpired = error != null && toAppError(error).code === "authExpired";

  return (
    <Section
      title={t("vpn.mineded.title")}
      actions={running ? <Badge tone="accent">{t("vpn.mineded.connected")}</Badge> : null}
    >
      <p className="text-sm text-muted">{t("vpn.mineded.intro")}</p>
      {accounts.isPending ? (
        <ListSkeleton rows={1} rowClassName="h-10" />
      ) : !account ? (
        <div className="flex flex-wrap items-center gap-3">
          <p className="flex-1 text-sm text-warning">{t("vpn.mineded.needAccount")}</p>
          <OpenAccounts />
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {candidates.length > 1 ? (
            <Field label={t("vpn.mineded.account")}>
              {(id) => (
                <Select id={id} value={account.id} onChange={(e) => setPicked(e.target.value)}>
                  {candidates.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name}
                    </option>
                  ))}
                </Select>
              )}
            </Field>
          ) : (
            <p className="text-sm">
              <span className="text-muted">{t("vpn.mineded.account")}: </span>
              <span className="font-medium">{account.name}</span>
            </p>
          )}
          {data ? <Usage access={data} /> : null}
          <div className="flex flex-wrap gap-2">
            {running ? (
              <Button
                icon={<Icon name="stop" />}
                loading={setMode.isPending}
                onClick={() => setMode.mutate("off")}
              >
                {t("vpn.mineded.disconnect")}
              </Button>
            ) : (
              <Button
                variant="primary"
                icon={<Icon name="shield" />}
                loading={connect.isPending}
                disabled={exhausted}
                onClick={() => {
                  setMode.reset();
                  connect.mutate(account.id);
                }}
              >
                {t("vpn.mineded.connect")}
              </Button>
            )}
          </div>
          <ErrorMessage error={error} />
          {authExpired ? (
            <div>
              <OpenAccounts />
            </div>
          ) : null}
        </div>
      )}
    </Section>
  );
}
