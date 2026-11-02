import { useTranslation } from "react-i18next";

import type { ManualDownload } from "../../bindings";
import { Icon } from "../../components/ui/Icon";
import { openExternal } from "../../lib/openExternal";

/** Pack files the launcher could not download: a link to that exact file. */
export function ManualDownloads({ items }: { items: ManualDownload[] }) {
  const { t } = useTranslation();
  if (items.length === 0) return null;
  return (
    <div className="rounded-xl border border-warning/40 bg-panel p-3 text-sm" data-testid="manual-downloads">
      <p className="mb-2 font-medium">{t("content.manual.title", { count: items.length })}</p>
      <ul className="flex flex-col gap-1">
        {items.map((m) => (
          <li key={m.fileName} className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate" title={m.fileName}>
              {m.title ?? m.fileName}
            </span>
            <button
              type="button"
              className="inline-flex shrink-0 cursor-pointer items-center gap-1 text-xs text-accent hover:underline"
              onClick={() => void openExternal(m.url)}
            >
              <Icon name="external" className="size-3.5" />
              {t("content.manual.open")}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
