import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import { useContentList, useInstances } from "../../api/queries";
import type { Instance } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { BORDERLESS_TITLES, findBorderlessMod, installedBorderless } from "../../lib/borderless";
import { useNavStore } from "../../stores/navStore";
import { type InstallRequest, InstallPlanDialog } from "../content/InstallPlanDialog";
import { pickInstance } from "../play/PlayScreen";

function ModdedHelper({ instance }: { instance: Instance }) {
  const { t } = useTranslation();
  const content = useContentList(instance.id);
  const [request, setRequest] = useState<InstallRequest | null>(null);
  const find = useMutation({
    mutationFn: () =>
      findBorderlessMod(instance, async (projectId, filter) => unwrap(await api.contentVersions("modrinth", projectId, filter))),
    onSuccess: (pick) => {
      if (pick) setRequest({ picks: [pick], titles: BORDERLESS_TITLES, shaders: false });
    },
  });
  const installed = installedBorderless(instance, content.data ?? []);

  if (installed) {
    return (
      <p className="flex flex-wrap items-center gap-2 text-xs text-muted" data-testid="borderless-installed">
        <Badge tone="accent">{t("voice.borderless.installed")}</Badge>
        {t("voice.borderless.useF11")}
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-1.5">
      <Button size="sm" variant="primary" className="w-fit" icon={<Icon name="maximize" />} loading={find.isPending} onClick={() => find.mutate()}>
        {t("voice.borderless.install")}
      </Button>
      {find.data === null ? <p className="text-xs text-warning">{t("voice.borderless.noVersion")}</p> : null}
      <ErrorMessage error={find.error} />
      <InstallPlanDialog instance={instance} installed={content.data ?? []} request={request} onClose={() => setRequest(null)} />
    </div>
  );
}

/**
 * Exclusive fullscreen hides the overlay: explains it and offers a one-click
 * borderless-fullscreen mod for the current instance (manual steps for vanilla).
 */
export function BorderlessHelper() {
  const { t } = useTranslation();
  const instances = useInstances();
  const playInstance = useNavStore((s) => s.playInstance);
  const instance = pickInstance(instances.data ?? [], playInstance);
  return (
    <div role="note" className="flex flex-col gap-2 rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm" data-testid="borderless-helper">
      <p className="flex items-start gap-2 font-medium text-warning">
        <Icon name="alert" className="mt-0.5 size-4 shrink-0" />
        {t("voice.borderless.title")}
      </p>
      <p className="text-xs text-text">{t("voice.borderless.hint")}</p>
      {instance ? (
        <>
          <p className="text-xs text-muted">{t("voice.borderless.forInstance", { name: instance.name, mc: instance.mc, loader: t(`loader.${instance.loader.kind}`) })}</p>
          {instance.loader.kind === "vanilla" ? (
            <p className="text-xs text-text" data-testid="borderless-vanilla">
              {t("voice.borderless.vanilla")}
            </p>
          ) : (
            <ModdedHelper key={instance.id} instance={instance} />
          )}
        </>
      ) : null}
    </div>
  );
}
