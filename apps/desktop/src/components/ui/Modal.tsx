import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useEffect, useId } from "react";
import { useTranslation } from "react-i18next";

import { useMotionPrefs, useMotionPreset } from "../../hooks/useAppearance";
import { Button } from "./Button";

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}

export function Modal({ open, onClose, title, children, footer, wide }: ModalProps) {
  const { light } = useMotionPrefs();
  const backdrop = useMotionPreset("backdrop");
  const dialog = useMotionPreset("dialog");
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open ? (
        <motion.div
          className={`fixed inset-0 z-50 flex items-center justify-center bg-overlay p-4 sm:p-6 ${light ? "" : "backdrop-blur-sm"}`}
          {...backdrop}
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) onClose();
          }}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            className={`flex max-h-full w-full flex-col overflow-hidden rounded-2xl border border-border bg-surface shadow-2xl ${wide ? "max-w-2xl" : "max-w-md"}`}
            {...dialog}
          >
            <h2 id={titleId} className="px-6 pt-5 pb-3 text-lg font-semibold">
              {title}
            </h2>
            <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-4">{children}</div>
            {footer ? <div className="flex justify-end gap-2 border-t border-border px-6 py-4">{footer}</div> : null}
          </motion.div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

export interface ConfirmDialogProps {
  open: boolean;
  title: ReactNode;
  message: ReactNode;
  confirmLabel: ReactNode;
  onConfirm: () => void;
  onCancel: () => void;
  loading?: boolean;
  danger?: boolean;
  error?: ReactNode;
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  onConfirm,
  onCancel,
  loading,
  danger,
  error,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  return (
    <Modal
      open={open}
      onClose={onCancel}
      title={title}
      footer={
        <>
          <Button variant="ghost" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button variant={danger ? "danger" : "primary"} loading={loading} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <p className="text-sm text-muted">{message}</p>
      {error ? <div className="mt-3">{error}</div> : null}
    </Modal>
  );
}
