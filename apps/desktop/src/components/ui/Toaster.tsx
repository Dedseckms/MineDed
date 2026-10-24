import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { useMotionPrefs, useMotionPreset } from "../../hooks/useAppearance";
import { errorView } from "../../lib/errors";
import { TOAST_TTL_MS, type Toast, type ToastTone, useToastStore } from "../../stores/toastStore";
import { Button } from "./Button";
import { ErrorDetails, ProxyHintActions } from "./ErrorDetails";
import { Icon } from "./Icon";

const TONE: Record<ToastTone, string> = {
  info: "border-info/40",
  warning: "border-warning/40",
  danger: "border-danger/40",
};

const ICON_TONE: Record<ToastTone, string> = {
  info: "text-info",
  warning: "text-warning",
  danger: "text-danger",
};

function ToastItem({ toast }: { toast: Toast }) {
  const { t, i18n } = useTranslation();
  const dismiss = useToastStore((s) => s.dismiss);
  const { reduced } = useMotionPrefs();
  const preset = useMotionPreset("toast");
  // Expanded details keep the toast on screen until it is closed.
  const [pinned, setPinned] = useState(false);
  const view = toast.error ? errorView(toast.error) : null;

  useEffect(() => {
    if (pinned) return;
    const timer = setTimeout(() => dismiss(toast.id), TOAST_TTL_MS);
    return () => clearTimeout(timer);
  }, [toast.id, dismiss, pinned]);

  return (
    <motion.div
      layout={!reduced}
      role={toast.tone === "info" ? "status" : "alert"}
      {...preset}
      className={`pointer-events-auto flex w-80 max-w-[calc(100vw-2rem)] items-start gap-3 rounded-xl border bg-surface p-3 shadow-xl ${TONE[toast.tone]}`}
    >
      <span className={`mt-0.5 ${ICON_TONE[toast.tone]}`}>
        <Icon name="alert" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <p className="text-sm font-medium break-words">{t(toast.messageKey, toast.params)}</p>
        {toast.detailKey && i18n.exists(toast.detailKey) ? (
          <p className="text-xs text-muted">{t(toast.detailKey, { ...view?.params, ...toast.params })}</p>
        ) : null}
        {view?.suggestProxy ? <ProxyHintActions onNavigate={() => dismiss(toast.id)} /> : null}
        {view ? <ErrorDetails view={view} onToggle={(open) => open && setPinned(true)} /> : null}
        {toast.action ? (
          <div>
            <Button
              size="sm"
              variant="primary"
              className="mt-1"
              onClick={() => {
                toast.action?.run();
                dismiss(toast.id);
              }}
            >
              {t(toast.action.labelKey)}
            </Button>
          </div>
        ) : null}
      </div>
      <Button size="sm" variant="ghost" className="h-7 px-1.5" aria-label={t("common.close")} onClick={() => dismiss(toast.id)} icon={<Icon name="x" />} />
    </motion.div>
  );
}

/** Bottom-right stack of transient notifications (no blur: safe for light mode). */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-40 flex flex-col items-end gap-2">
      <AnimatePresence initial={false}>
        {toasts.map((toast) => (
          <ToastItem key={toast.id} toast={toast} />
        ))}
      </AnimatePresence>
    </div>
  );
}
