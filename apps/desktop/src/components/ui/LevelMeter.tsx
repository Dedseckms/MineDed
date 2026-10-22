import { meterPercent } from "../../lib/voice";

/** Horizontal audio level meter (0..1). Width changes only, no animation loop in JS. */
export function LevelMeter({ level, label, className = "" }: { level: number; label: string; className?: string }) {
  const pct = meterPercent(level);
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      className={`h-2 w-full overflow-hidden rounded-full bg-surface-2 ${className}`}
    >
      <div
        className={`h-full rounded-full transition-[width] duration-100 ${pct > 85 ? "bg-warning" : "bg-accent"}`}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export interface RangeProps {
  label: string;
  /** Percent 0..200. */
  value: number;
  onChange: (percent: number) => void;
  disabled?: boolean;
  className?: string;
  /** Visible label next to the slider. */
  showLabel?: boolean;
}

/** 0–200 % slider with the value shown on the right. */
export function PercentSlider({ label, value, onChange, disabled, className = "", showLabel = true }: RangeProps) {
  return (
    <div className={`flex items-center gap-3 ${className}`}>
      {showLabel ? <span className="w-40 shrink-0 text-sm font-medium">{label}</span> : null}
      <input
        type="range"
        min={0}
        max={200}
        step={5}
        value={value}
        disabled={disabled}
        aria-label={label}
        aria-valuetext={`${value}%`}
        onChange={(e) => onChange(Number(e.target.value))}
        className="min-w-0 flex-1 cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
      />
      <span className="w-12 shrink-0 text-right text-xs text-muted tabular-nums">{value}%</span>
    </div>
  );
}
