import { type ReactNode, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { VpnApp } from "../../bindings";
import { pickFile } from "../../api/dialog";
import { useSetVpnApps, useSystemInfo, useVpnApps, useVpnRunningApps } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { VirtualList } from "../../components/ui/VirtualList";
import { ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { TextInput } from "../../components/ui/form";
import { APP_PRESETS, addApp, appFromPath, filterApps, hasApp, presetApp, removeApp, sameApps } from "../../lib/vpn";

const ROW = 48;
const VIEWPORT = 288;

function AppRow({ app, action }: { app: VpnApp; action: ReactNode }) {
  return (
    <div className="mx-1 flex h-[42px] items-center gap-3 rounded-lg px-3 text-sm hover:bg-surface-2">
      <span className="min-w-0 flex-1 truncate font-medium" title={app.path ?? undefined}>
        {app.name}
      </span>
      <span className="min-w-0 max-w-[45%] truncate font-mono text-xs text-muted" title={app.process}>
        {app.process}
      </span>
      {action}
    </div>
  );
}

function RunningPicker({
  open,
  onClose,
  list,
  onAdd,
}: {
  open: boolean;
  onClose: () => void;
  list: VpnApp[];
  onAdd: (app: VpnApp) => void;
}) {
  const { t } = useTranslation();
  const running = useVpnRunningApps(open);
  const [query, setQuery] = useState("");
  const visible = useMemo(() => filterApps(running.data ?? [], query), [running.data, query]);
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("vpn.apps.runningTitle")}
      footer={
        <Button variant="primary" onClick={onClose}>
          {t("common.close")}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <TextInput
          type="search"
          aria-label={t("vpn.apps.search")}
          placeholder={t("vpn.apps.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {running.isPending ? (
          <ListSkeleton rows={4} rowClassName="h-[42px]" />
        ) : running.error ? (
          <ErrorMessage error={running.error} onRetry={() => void running.refetch()} />
        ) : visible.length === 0 ? (
          <p className="text-sm text-muted">{t("vpn.apps.noRunning")}</p>
        ) : (
          <VirtualList
            items={visible}
            rowHeight={ROW}
            height={VIEWPORT}
            getKey={(a) => `${a.process}|${a.path ?? ""}`}
            renderRow={(a) => {
              const added = hasApp(list, a);
              return (
                <AppRow
                  app={a}
                  action={
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={added}
                      icon={<Icon name={added ? "check" : "plus"} />}
                      aria-label={added ? undefined : t("vpn.apps.add", { name: a.name })}
                      onClick={() => onAdd(a)}
                    >
                      {added ? t("vpn.apps.added") : null}
                    </Button>
                  }
                />
              );
            }}
          />
        )}
      </div>
    </Modal>
  );
}

/** Editor of the programs routed through the VPN in `apps` mode. */
export function AppsSection() {
  const { t } = useTranslation();
  const saved = useVpnApps();
  const save = useSetVpnApps();
  const sys = useSystemInfo();
  const os = sys.data?.os ?? "windows";
  const [draft, setDraft] = useState<VpnApp[] | null>(null);
  const [picker, setPicker] = useState(false);
  const [pickError, setPickError] = useState<unknown>(null);

  const list = draft ?? saved.data ?? [];
  const dirty = draft !== null && !sameApps(draft, saved.data ?? []);
  const edit = (next: VpnApp[]) => {
    save.reset();
    setDraft(next);
  };

  const fromFile = async () => {
    setPickError(null);
    try {
      const path = await pickFile(os === "windows" ? [{ name: t("dialog.exe"), extensions: ["exe"] }] : []);
      const app = path ? appFromPath(path) : null;
      if (app) edit(addApp(list, app));
    } catch (e) {
      setPickError(e);
    }
  };

  return (
    <Section
      title={t("vpn.apps.title")}
      actions={
        <Button
          size="sm"
          variant="primary"
          icon={<Icon name="check" />}
          disabled={!dirty}
          loading={save.isPending}
          onClick={() => save.mutate(list, { onSuccess: () => setDraft(null) })}
        >
          {t("vpn.apps.save")}
        </Button>
      }
    >
      <p className="text-xs text-muted">{t("vpn.apps.hint")}</p>
      <div className="flex flex-col gap-2">
        <span className="text-sm font-medium">{t("vpn.apps.presets")}</span>
        <div className="flex flex-wrap gap-2">
          {APP_PRESETS.map((p) => {
            const app = presetApp(p, os);
            const added = hasApp(list, app);
            return (
              <Button
                key={p.name}
                size="sm"
                disabled={added}
                icon={<Icon name={added ? "check" : "plus"} />}
                aria-label={t("vpn.apps.addPreset", { name: p.name })}
                onClick={() => edit(addApp(list, app))}
              >
                {p.name}
              </Button>
            );
          })}
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" icon={<Icon name="search" />} onClick={() => setPicker(true)}>
          {t("vpn.apps.running")}
        </Button>
        <Button size="sm" icon={<Icon name="folder" />} onClick={() => void fromFile()}>
          {t("vpn.apps.pickFile")}
        </Button>
      </div>
      {saved.isPending ? (
        <ListSkeleton rows={2} rowClassName="h-[42px]" />
      ) : saved.error ? (
        <ErrorMessage error={saved.error} onRetry={() => void saved.refetch()} />
      ) : list.length === 0 ? (
        <p className="text-sm text-muted">{t("vpn.apps.empty")}</p>
      ) : (
        <VirtualList
          items={list}
          rowHeight={ROW}
          height={VIEWPORT}
          ariaLabel={t("vpn.apps.label")}
          getKey={(a) => a.process.toLowerCase()}
          renderRow={(a) => (
            <AppRow
              app={a}
              action={
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<Icon name="x" />}
                  aria-label={t("vpn.apps.remove", { name: a.name })}
                  onClick={() => edit(removeApp(list, a.process))}
                />
              }
            />
          )}
        />
      )}
      <ErrorMessage error={save.error ?? pickError} />
      <RunningPicker open={picker} onClose={() => setPicker(false)} list={list} onAdd={(a) => edit(addApp(list, a))} />
    </Section>
  );
}
