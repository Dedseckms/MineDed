import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { errorKey, isNotImplemented } from "../../lib/errors";

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
export function ErrorMessage({ error, className = "" }: { error: unknown; className?: string }) {
  const { t } = useTranslation();
  if (error == null) return null;
  const soft = isNotImplemented(error);
  return (
    <div
      role={soft ? "status" : "alert"}
      className={`rounded-lg border px-3 py-2 text-sm ${
        soft ? "border-warning/30 bg-warning/10 text-warning" : "border-danger/30 bg-danger/10 text-danger"
      } ${className}`}
    >
      {t(errorKey(error))}
    </div>
  );
}

export function EmptyState({ title, children }: { title: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border p-10 text-center">
      <p className="font-medium">{title}</p>
      {children}
    </div>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div className={`animate-pulse rounded-lg bg-surface-2 ${className}`} />;
}
