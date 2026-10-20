import { useId } from "react";
import { useTranslation } from "react-i18next";

import type { MemoryCfg } from "../../bindings";
import { Switch } from "../../components/ui/form";
import { MEM_MIN_MB, MEM_STEP_MB, type MemoryErrors } from "../../lib/validation";

export const FALLBACK_RAM_MB = 16_384;

/** Megabytes → `{ value, unitKey }` (GB above 1024 MB). */
export function splitMb(mb: number): { value: string; unitKey: "units.mb" | "units.gb" } {
  if (mb < 1024) return { value: String(mb), unitKey: "units.mb" };
  return { value: (mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1), unitKey: "units.gb" };
}

export function useFormatMb() {
  const { t } = useTranslation();
  return (mb: number) => {
    const { value, unitKey } = splitMb(mb);
    return t(unitKey, { value });
  };
}

interface SliderProps {
  label: string;
  value: number;
  max: number;
  disabled?: boolean;
  error?: string;
  onChange: (v: number) => void;
}

function MemSlider({ label, value, max, disabled, error, onChange }: SliderProps) {
  const { t } = useTranslation();
  const formatMb = useFormatMb();
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between text-sm">
        <label htmlFor={id} className="font-medium">
          {label}
        </label>
        <span className="tabular-nums text-muted">{formatMb(value)}</span>
      </div>
      <input
        id={id}
        type="range"
        min={MEM_MIN_MB}
        max={Math.max(max, value)}
        step={MEM_STEP_MB}
        value={value}
        disabled={disabled}
        aria-invalid={!!error || undefined}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
      />
      {error ? (
        <p role="alert" className="text-xs text-danger">
          {t(error)}
        </p>
      ) : null}
    </div>
  );
}

export interface MemoryEditorProps {
  value: MemoryCfg;
  onChange: (v: MemoryCfg) => void;
  totalRamMb: number | null;
  errors: MemoryErrors;
  hint?: string | null;
}

/** Min/max RAM sliders + "auto" switch (auto recalculates max on every launch). */
export function MemoryEditor({ value, onChange, totalRamMb, errors, hint }: MemoryEditorProps) {
  const { t } = useTranslation();
  const max = totalRamMb ?? FALLBACK_RAM_MB;
  return (
    <div className="flex flex-col gap-4">
      <Switch
        checked={value.auto}
        onChange={(auto) => onChange({ ...value, auto })}
        label={t("memory.auto")}
        description={t("memory.autoHint")}
      />
      <MemSlider
        label={t("memory.min")}
        value={value.minMb}
        max={max}
        error={errors.minMb}
        onChange={(minMb) => onChange({ ...value, minMb })}
      />
      <MemSlider
        label={t("memory.max")}
        value={value.maxMb}
        max={max}
        disabled={value.auto}
        error={errors.maxMb}
        onChange={(maxMb) => onChange({ ...value, maxMb })}
      />
      {hint ? <p className="text-xs text-muted">{hint}</p> : null}
    </div>
  );
}
