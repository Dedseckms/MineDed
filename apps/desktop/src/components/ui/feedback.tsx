import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { errorKey, isNotImplemented } from "../../lib/errors";
import { Button } from "./Button";
import { Icon, type IconName } from "./Icon";

type Tone = "neutral" | "accent" | "danger" | "warning" | "info";

const BADGE: Record<Tone, string> = {
  neutral: "bg-surface-2 text-muted",
  accent: "bg-accent/15 text-accent",
  danger: "bg-danger/15 text-danger",
  warning: "bg-warning/15 text-warning",
  info: "bg-info/15 text-info",
};

export function Badge({ tone = "neutral", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ${BADGE[tone]}`}>
      {children}
    </span>
  );
}

/** Shows an error by its `errors.<code>` key; `notImplemented` is a soft notice. */
export function ErrorMessage({
  error,
  className = "",
  onRetry,
}: {
  error: unknown;
  className?: string;
  /** Shows a "Retry" button (for failed queries). */
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  if (error == null) return null;
  const soft = isNotImplemented(error);
  return (
    <div
      role={soft ? "status" : "alert"}
      className={`flex items-center gap-3 rounded-lg border px-3 py-2 text-sm ${
        soft ? "border-warning/30 bg-warning/10 text-warning" : "border-danger/30 bg-danger/10 text-danger"
      } ${className}`}
    >
      <span className="min-w-0 flex-1 break-words">{t(errorKey(error))}</span>
      {onRetry ? (
        <Button size="sm" variant="ghost" onClick={onRetry}>
          {t("common.retry")}
        </Button>
      ) : null}
    </div>
  );
}

export function EmptyState({
  title,
  description,
  icon,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  icon?: IconName;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border px-6 py-10 text-center">
      {icon ? (
        <span className="grid size-10 place-items-center rounded-full bg-surface-2 text-muted" aria-hidden>
          <Icon name={icon} />
        </span>
      ) : null}
      <p className="font-medium">{title}</p>
      {description ? <p className="max-w-prose text-sm text-muted">{description}</p> : null}
      {children}
    </div>
  );
}

/** Placeholder block; static (no pulse) when motion is reduced via CSS. */
export function Skeleton({ className = "" }: { className?: string }) {
  return <div aria-hidden className={`animate-pulse rounded-lg bg-surface-2 ${className}`} />;
}

/** Loading placeholder for a list: `rows` blocks in a column or grid. */
export function ListSkeleton({
  rows = 4,
  rowClassName = "h-16",
  className = "flex flex-col gap-2",
}: {
  rows?: number;
  rowClassName?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <div role="status" aria-busy="true" aria-label={t("common.loading")} className={className} data-testid="skeleton">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className={rowClassName} />
      ))}
    </div>
  );
}