import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { formatBytes } from "../../lib/content";
import { openExternal } from "../../lib/openExternal";

/** Link opened in the system browser, plus a copy button. */
export function ExternalLink({ href, children }: { href: string; children: ReactNode }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <span className="inline-flex min-w-0 items-center gap-1">
      <button
        type="button"
        onClick={() => void openExternal(href)}
        className="inline-flex min-w-0 cursor-pointer items-center gap-1 text-accent underline-offset-2 hover:underline"
      >
        <span className="truncate">{children}</span>
        <Icon name="external" className="size-3.5 shrink-0" />
      </button>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 px-1.5 text-xs"
        onClick={() => {
          void navigator.clipboard
            ?.writeText(href)
            .then(() => setCopied(true))
            .catch(() => setCopied(false));
        }}
      >
        {copied ? t("common.copied") : t("common.copyLink")}
      </Button>
    </span>
  );
}

export function useFormatBytes() {
  const { t, i18n } = useTranslation();
  return (bytes: number) => {
    const { key, value } = formatBytes(bytes, i18n.language);
    return t(key, { value });
  };
}
