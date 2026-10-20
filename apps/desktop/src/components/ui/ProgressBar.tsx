import { motion } from "motion/react";

import { useMotionPrefs } from "../../hooks/useAppearance";

export interface ProgressBarProps {
  /** 0..1, or `null` for an indeterminate bar. */
  value: number | null;
  tone?: "accent" | "danger" | "muted";
  className?: string;
  label?: string;
}

const TONES = { accent: "bg-accent", danger: "bg-danger", muted: "bg-muted" } as const;

/** Smoothly animated progress (instant when motion is reduced). */
export function ProgressBar({ value, tone = "accent", className = "", label }: ProgressBarProps) {
  const { reduced } = useMotionPrefs();
  const pct = value === null ? undefined : Math.round(value * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      className={`relative h-1.5 w-full overflow-hidden rounded-full bg-surface-2 ${className}`}
    >
      {value === null ? (
        reduced ? (
          <div className={`absolute inset-0 opacity-40 ${TONES[tone]}`} />
        ) : (
          <div className={`progress-indeterminate absolute inset-y-0 w-2/5 rounded-full ${TONES[tone]}`} />
        )
      ) : (
        <motion.div
          className={`absolute inset-y-0 left-0 w-full origin-left rounded-full ${TONES[tone]}`}
          initial={false}
          animate={{ scaleX: value }}
          transition={reduced ? { duration: 0 } : { type: "spring", stiffness: 120, damping: 24 }}
        />
      )}
    </div>
  );
}
