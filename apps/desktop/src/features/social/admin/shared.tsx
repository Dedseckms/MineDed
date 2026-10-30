import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

/** One moderation/admin action; every `social` query is refreshed on success. */
export function useSocialAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (run: () => Promise<unknown>) => run(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["social"] }),
  });
}

/**
 * Large admin layout: list on the left, details on the right on wide windows;
 * on narrow ones the details move under the list and the page scrolls.
 */
export function AdminSplit({
  list,
  details,
}: {
  list: ReactNode;
  details: ReactNode;
}) {
  return (
    <div className="grid min-h-0 flex-1 gap-3 overflow-y-auto lg:grid-cols-[minmax(0,20rem)_minmax(0,1fr)] lg:overflow-hidden">
      <section className="flex min-h-0 flex-col overflow-hidden rounded-xl border border-border bg-panel">
        {list}
      </section>
      <section className="min-h-0 overflow-y-auto rounded-xl border border-border bg-panel p-4">
        {details}
      </section>
    </div>
  );
}

export interface ListRowProps {
  active?: boolean;
  onClick?: () => void;
  children: ReactNode;
}

/** Clickable row of the left column, highlighted when selected. */
export function ListRow({ active = false, onClick, children }: ListRowProps) {
  return (
    <button
      type="button"
      aria-current={active ? "true" : undefined}
      onClick={onClick}
      className={`flex w-full cursor-pointer flex-col gap-0.5 border-b border-border px-3 py-2 text-left transition-colors ${
        active ? "bg-accent/10" : "hover:bg-surface-2"
      }`}
    >
      {children}
    </button>
  );
}
