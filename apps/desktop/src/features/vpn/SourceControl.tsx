import { useTranslation } from "react-i18next";

import type { VpnSource, VpnState } from "../../bindings";
import { useSetVpnSource } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Badge, ErrorMessage } from "../../components/ui/feedback";

/** Which subscription is active: "In use" badge, or "Use" on the configured inactive one. */
export function SourceControl({ which, state }: { which: VpnSource; state: VpnState | undefined }) {
  const { t } = useTranslation();
  const setSource = useSetVpnSource();
  const active = state?.source === which;
  const configured = which === "mineded" ? !!state?.hasMineded : !!state?.hasOwn;
  if (active) return <Badge tone="accent">{t("vpn.source.active")}</Badge>;
  if (!configured) return <Badge>{t("vpn.source.notSet")}</Badge>;
  return (
    <div className="flex flex-col items-end gap-1">
      <Button
        size="sm"
        variant="primary"
        loading={setSource.isPending}
        aria-label={t("vpn.source.useNamed", { name: t(`vpn.source.${which}`) })}
        onClick={() => setSource.mutate(which)}
      >
        {t("vpn.source.use")}
      </Button>
      <ErrorMessage error={setSource.error} />
    </div>
  );
}
