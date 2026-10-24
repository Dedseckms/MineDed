import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ErrorView } from "../../lib/errors";
import { useNavStore } from "../../stores/navStore";
import { Button } from "./Button";
import { CopyButton } from "./CopyButton";
import { Icon } from "./Icon";

/** "Turn on a proxy or VPN" shortcuts for `details.hint = useProxy`. */
export function ProxyHintActions({ onNavigate }: { onNavigate?: () => void }) {
  const { t } = useTranslation();
  const go = useNavStore((s) => s.go);
  return (
    <div className="flex flex-wrap items-center gap-2" data-testid="proxy-hint">
      <span className="text-xs font-medium">{t("errors.hint.useProxy")}</span>
      <Button
        size="sm"
        variant="secondary"
        icon={<Icon name="shield" />}
        onClick={() => {
          go("vpn");
          onNavigate?.();
        }}
      >
        {t("errors.hint.openVpn")}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        icon={<Icon name="settings" />}
        onClick={() => {
          go("settings");
          onNavigate?.();
        }}
      >
        {t("errors.hint.openProxy")}
      </Button>
    </div>
  );
}

/**
 * Collapsible technical details of an `AppError`: code, details and a copy
 * button (for bug reports). Text stays selectable.
 */
export function ErrorDetails({ view, onToggle }: { view: ErrorView; onToggle?: (open: boolean) => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <div className="flex flex-col gap-1.5 text-xs">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
          onToggle?.(!open);
        }}
        className="inline-flex w-fit cursor-pointer items-center gap-1 rounded-md font-medium opacity-90 hover:underline"
      >
        <Icon name="chevronDown" className={`size-3.5 transition-transform ${open ? "rotate-180" : ""}`} />
        {open ? t("errors.less") : t("errors.more")}
      </button>
      {open ? (
        <div className="selectable flex flex-col gap-2 rounded-md border border-border bg-surface p-2 text-text">
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 font-mono break-all">
            <dt className="text-muted">{t("errors.code")}</dt>
            <dd>{view.code}</dd>
            {view.details.map(([k, v]) => (
              <div key={k} className="contents">
                <dt className="text-muted">{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
          </dl>
          <CopyButton value={view.copyText} className="w-fit" />
        </div>
      ) : null}
    </div>
  );
}
