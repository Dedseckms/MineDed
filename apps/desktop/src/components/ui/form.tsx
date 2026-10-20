import { type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, useId } from "react";
import { useTranslation } from "react-i18next";

const CONTROL =
  "h-10 w-full rounded-lg border bg-surface-2 px-3 text-sm text-text outline-none transition-colors placeholder:text-muted focus:border-accent disabled:opacity-50";

export interface FieldProps {
  label: ReactNode;
  /** i18n key of a validation error. */
  error?: string | null;
  hint?: ReactNode;
  children: (id: string, describedBy: string | undefined, invalid: boolean) => ReactNode;
  className?: string;
}

/** Label + control + hint/error, wired for a11y. */
export function Field({ label, error, hint, children, className = "" }: FieldProps) {
  const { t } = useTranslation();
  const id = useId();
  const msgId = `${id}-msg`;
  const invalid = !!error;
  return (
    <div className={`flex flex-col gap-1.5 ${className}`}>
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      {children(id, error || hint ? msgId : undefined, invalid)}
      {error ? (
        <p id={msgId} role="alert" className="text-xs text-danger">
          {t(error)}
        </p>
      ) : hint ? (
        <p id={msgId} className="text-xs text-muted">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  invalid?: boolean;
}

export function TextInput({ invalid, className = "", ...rest }: TextInputProps) {
  return (
    <input
      aria-invalid={invalid || undefined}
      className={`${CONTROL} ${invalid ? "border-danger" : "border-border"} ${className}`}
      spellCheck={false}
      autoComplete="off"
      {...rest}
    />
  );
}

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  invalid?: boolean;
}

export function Select({ invalid, className = "", children, ...rest }: SelectProps) {
  return (
    <select
      aria-invalid={invalid || undefined}
      className={`${CONTROL} cursor-pointer ${invalid ? "border-danger" : "border-border"} ${className}`}
      {...rest}
    >
      {children}
    </select>
  );
}

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}

export function Switch({ checked, onChange, label, description, disabled }: SwitchProps) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex flex-col gap-0.5">
        <label htmlFor={id} className="cursor-pointer text-sm font-medium">
          {label}
        </label>
        {description ? <p className="text-xs text-muted">{description}</p> : null}
      </div>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative h-6 w-11 shrink-0 cursor-pointer rounded-full transition-colors disabled:opacity-50 ${checked ? "bg-accent" : "bg-border"}`}
      >
        <span
          className={`absolute top-0.5 left-0.5 size-5 rounded-full bg-white shadow transition-transform ${checked ? "translate-x-5" : ""}`}
        />
      </button>
    </div>
  );
}

export interface SegmentedProps<T extends string> {
  value: T;
  options: Array<{ value: T; label: ReactNode; disabled?: boolean; badge?: ReactNode }>;
  onChange: (value: T) => void;
  ariaLabel: string;
}

export function Segmented<T extends string>({ value, options, onChange, ariaLabel }: SegmentedProps<T>) {
  return (
    <div role="radiogroup" aria-label={ariaLabel} className="inline-flex flex-wrap gap-1 rounded-lg bg-surface-2 p-1">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={`inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md px-3 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
            o.value === value ? "bg-surface text-text shadow-sm" : "text-muted hover:text-text"
          }`}
        >
          {o.label}
          {o.badge}
        </button>
      ))}
    </div>
  );
}
