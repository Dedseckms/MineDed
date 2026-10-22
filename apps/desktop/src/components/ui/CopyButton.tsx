import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Icon } from "./Icon";

/** Copies `value` to the clipboard and briefly shows "Copied". */
export function CopyButton({ value, label, className = "" }: { value: string; label?: string; className?: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <Button
      size="sm"
      variant="secondary"
      className={className}
      icon={<Icon name={copied ? "check" : "copy"} />}
      onClick={() => {
        void navigator.clipboard
          ?.writeText(value)
          .then(() => setCopied(true))
          .catch(() => setCopied(false));
      }}
    >
      {copied ? t("common.copied") : (label ?? t("common.copy"))}
    </Button>
  );
}
