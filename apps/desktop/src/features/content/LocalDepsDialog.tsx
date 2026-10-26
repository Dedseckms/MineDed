import { useTranslation } from "react-i18next";

import type { LocalDepIssue } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Modal } from "../../components/ui/Modal";
import { Badge } from "../../components/ui/feedback";
import { groupIssuesByFile, suggestedPicks } from "../../lib/content";
import type { InstallRequest } from "./InstallPlanDialog";

const TONE = { missing: "warning", wrongVersion: "warning", conflict: "danger", wrongLoader: "danger" } as const;

/**
 * Result of `content_check_local`: issues grouped by jar; suggestions go to
 * the regular install flow (`InstallPlanDialog` resolves them).
 */
export function LocalDepsDialog({
  issues,
  onClose,
  onInstall,
}: {
  /** `null` = closed; empty list = "all good". */
  issues: LocalDepIssue[] | null;
  onClose: () => void;
  onInstall: (r: InstallRequest) => void;
}) {
  const { t } = useTranslation();
  const list = issues ?? [];
  const all = suggestedPicks(list);
  const install = (picked: LocalDepIssue[]) => {
    const { picks, titles } = suggestedPicks(picked);
    if (picks.length > 0) onInstall({ picks, titles, shaders: false });
  };
  return (
    <Modal
      open={issues !== null}
      onClose={onClose}
      wide
      title={t("content.localDeps.title")}
      footer={
        <>
          <Button onClick={onClose}>{t("common.close")}</Button>
          {all.picks.length > 0 ? (
            <Button variant="primary" icon={<Icon name="download" />} onClick={() => install(list)}>
              {t("content.localDeps.installAll", { n: all.picks.length })}
            </Button>
          ) : null}
        </>
      }
    >
      {list.length === 0 ? (
        <p className="flex items-center gap-2 text-sm" data-testid="local-deps-ok">
          <Icon name="check" className="size-4 text-accent" />
          {t("content.localDeps.ok")}
        </p>
      ) : (
        <div className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto">
          <p className="text-sm text-muted">{t("content.localDeps.intro")}</p>
          {groupIssuesByFile(list).map(([file, group]) => (
            <div key={file} className="flex flex-col gap-2 rounded-lg border border-border bg-surface-2 p-3" data-testid="local-deps-file">
              <p className="truncate font-mono text-sm font-medium">{file}</p>
              {group.map((i, k) => (
                <div key={`${i.kind}-${i.modId}-${k}`} className="flex flex-wrap items-center gap-2 text-sm">
                  <Badge tone={TONE[i.kind]}>{t(`content.localDeps.kind.${i.kind}`)}</Badge>
                  <span className="min-w-0 flex-1">
                    {t(`content.localDeps.detail.${i.kind}`, { mod: i.modId, req: i.versionReq ?? "*" })}
                  </span>
                  {i.suggestion ? (
                    <Button size="sm" icon={<Icon name="download" />} onClick={() => install([i])}>
                      {t("content.localDeps.install", { title: i.suggestionTitle ?? i.modId })}
                    </Button>
                  ) : null}
                </div>
              ))}
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}
