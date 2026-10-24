import { useTranslation } from "react-i18next";

import { manualLinks } from "../../lib/modpacks";
import { ExternalLink } from "./ExternalLink";

/** Download pages of a `manualDownloadRequired` error; renders nothing otherwise. */
export function ManualDownloadLinks({ error }: { error: unknown }) {
  const { t } = useTranslation();
  const links = manualLinks(error);
  if (links.length === 0) return null;
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm">
      <p className="font-medium text-warning">{t("modpacks.manual.title")}</p>
      <p className="text-xs text-muted">{t("modpacks.manual.hint")}</p>
      <ul className="flex flex-col gap-1">
        {links.map((l) => (
          <li key={l.key} className="min-w-0">
            <ExternalLink href={l.url}>{l.url}</ExternalLink>
          </li>
        ))}
      </ul>
    </div>
  );
}
