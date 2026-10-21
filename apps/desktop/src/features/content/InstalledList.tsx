import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { ContentKind, InstalledContent, Instance } from "../../bindings";
import { useRemoveContent, useSetContentEnabled } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { VirtualList } from "../../components/ui/VirtualList";
import { TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage } from "../../components/ui/feedback";
import { displayName, filterInstalled } from "../../lib/content";

const ROW_HEIGHT = 60;
const LIST_HEIGHT = 480;

function Row({
  item,
  onToggle,
  onRemove,
}: {
  item: InstalledContent;
  onToggle: (enabled: boolean) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const name = displayName(item);
  return (
    <div className={`flex h-full items-center gap-3 border-b border-border px-3 ${item.enabled ? "" : "opacity-60"}`}>
      <button
        type="button"
        role="switch"
        aria-checked={item.enabled}
        aria-label={t(item.enabled ? "content.disable" : "content.enable", { name })}
        onClick={() => onToggle(!item.enabled)}
        className={`relative h-5 w-9 shrink-0 cursor-pointer rounded-full transition-colors ${item.enabled ? "bg-accent" : "bg-border"}`}
      >
        <span className={`absolute top-0.5 left-0.5 size-4 rounded-full bg-white shadow transition-transform ${item.enabled ? "translate-x-4" : ""}`} />
      </button>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{name}</p>
        <p className="truncate text-xs text-muted">
          {item.versionNumber ? `${item.versionNumber} · ` : ""}
          {item.fileName}
        </p>
      </div>
      {item.updateAvailable ? <Badge tone="info">{t("content.updateAvailable")}</Badge> : null}
      <Badge>{t(`content.source.${item.source}`)}</Badge>
      <Button size="sm" variant="ghost" className="text-danger" icon={<Icon name="trash" />} aria-label={t("content.removeNamed", { name })} onClick={onRemove} />
    </div>
  );
}

/** Installed files of one kind: toggle, remove, filter; windowed for big packs. */
export function InstalledList({
  instance,
  kind,
  items,
  onBrowse,
}: {
  instance: Instance;
  kind: ContentKind;
  items: InstalledContent[];
  onBrowse: () => void;
}) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState("");
  const [removing, setRemoving] = useState<InstalledContent | null>(null);
  const toggle = useSetContentEnabled(instance.id);
  const remove = useRemoveContent(instance.id);
  const shown = filterInstalled(items, filter);
  const enabled = items.filter((c) => c.enabled).length;

  if (items.length === 0) {
    return (
      <EmptyState title={t(`content.emptyKind.${kind}`)}>
        <Button variant="primary" icon={<Icon name="search" />} onClick={onBrowse}>
          {t("content.browse")}
        </Button>
      </EmptyState>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <TextInput
          type="search"
          aria-label={t("content.filterInstalled")}
          placeholder={t("content.filterInstalled")}
          className="max-w-xs"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <span className="text-xs text-muted">{t("content.enabledCount", { enabled, total: items.length })}</span>
        <Button className="ml-auto" size="sm" icon={<Icon name="search" />} onClick={onBrowse}>
          {t("content.browse")}
        </Button>
      </div>
      <div className="rounded-xl border border-border bg-surface">
        {shown.length === 0 ? (
          <p className="p-4 text-sm text-muted">{t("content.noMatches")}</p>
        ) : (
          <VirtualList
            items={shown}
            rowHeight={ROW_HEIGHT}
            height={LIST_HEIGHT}
            ariaLabel={t(`content.kind.${kind}`)}
            getKey={(c) => c.fileName}
            renderRow={(c) => (
              <Row
                item={c}
                onToggle={(v) => toggle.mutate({ fileName: c.fileName, enabled: v })}
                onRemove={() => {
                  remove.reset();
                  setRemoving(c);
                }}
              />
            )}
          />
        )}
      </div>
      <ErrorMessage error={toggle.error} />
      <ConfirmDialog
        open={removing !== null}
        danger
        title={t("content.removeTitle")}
        message={t("content.removeMessage", { name: removing ? displayName(removing) : "" })}
        confirmLabel={t("common.delete")}
        loading={remove.isPending}
        error={remove.error ? <ErrorMessage error={remove.error} /> : null}
        onCancel={() => setRemoving(null)}
        onConfirm={() => {
          if (removing) remove.mutate(removing.fileName, { onSuccess: () => setRemoving(null) });
        }}
      />
    </div>
  );
}
