import type { ReactNode } from "react";

export interface TabsProps<T extends string> {
  value: T;
  tabs: Array<{ value: T; label: ReactNode; badge?: ReactNode }>;
  onChange: (value: T) => void;
  ariaLabel: string;
  idPrefix: string;
}

/** Underlined tab bar; panels use `aria-labelledby={`${idPrefix}-${value}`}`. */
export function Tabs<T extends string>({ value, tabs, onChange, ariaLabel, idPrefix }: TabsProps<T>) {
  return (
    <div role="tablist" aria-label={ariaLabel} className="flex gap-1 overflow-x-auto border-b border-border">
      {tabs.map((tab) => {
        const active = tab.value === value;
        return (
          <button
            key={tab.value}
            id={`${idPrefix}-${tab.value}`}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(tab.value)}
            className={`-mb-px inline-flex h-10 shrink-0 cursor-pointer items-center gap-2 border-b-2 px-3 text-sm font-medium transition-colors ${
              active ? "border-accent text-text" : "border-transparent text-muted hover:text-text"
            }`}
          >
            {tab.label}
            {tab.badge}
          </button>
        );
      })}
    </div>
  );
}
