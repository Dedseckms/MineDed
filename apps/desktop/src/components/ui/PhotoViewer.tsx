import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

import { useMotionPreset } from "../../hooks/useAppearance";
import { Icon } from "./Icon";

export interface PhotoViewerProps {
  /** Full-size image (`null` = closed). Previews arrive as bounded native data URLs. */
  src: string | null;
  alt: string;
  onClose: () => void;
  /** Saves the original attachment; the button is hidden without it. */
  onSave?: () => void;
  /** Accessible dialog title; defaults to the generic "photo" label. */
  title?: ReactNode;
}

const FOCUSABLE =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

/**
 * Full-screen photo viewer on a dark scrim. Closes on Esc and on a backdrop
 * click, traps Tab inside the dialog and never touches page scroll, so the chat
 * timeline underneath keeps its position. Rendered into `document.body`.
 */
export function PhotoViewer({ src, alt, onClose, onSave, title }: PhotoViewerProps) {
  const { t } = useTranslation();
  const backdrop = useMotionPreset("backdrop");
  const dialog = useMotionPreset("dialog");
  const titleId = useId();
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!src) return;
    const previous = document.activeElement as HTMLElement | null;
    root.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== "Tab" || !root.current) return;
      const nodes = [
        ...root.current.querySelectorAll<HTMLElement>(FOCUSABLE),
      ].filter((node) => !node.hasAttribute("disabled"));
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      if (!first || !last) return;
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !root.current.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      previous?.focus?.();
    };
  }, [src, onClose]);

  if (typeof document === "undefined") return null;

  return createPortal(
    <AnimatePresence>
      {src ? (
        <motion.div
          ref={root}
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
          className="fixed inset-0 z-[60] flex flex-col bg-black/90 text-white"
          {...backdrop}
        >
          <h2 id={titleId} className="sr-only">
            {title ?? t("social.photoViewer")}
          </h2>
          <div className="flex shrink-0 items-center justify-end gap-2 bg-gradient-to-b from-black/60 to-transparent px-3 py-2">
            {onSave ? (
              <ToolbarButton onClick={onSave} icon="download">
                {t("common.save")}
              </ToolbarButton>
            ) : null}
            <ToolbarButton onClick={onClose} icon="x" autoFocus>
              {t("common.close")}
            </ToolbarButton>
          </div>
          <div
            data-testid="photo-backdrop"
            className="flex min-h-0 flex-1 items-center justify-center p-3"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) onClose();
            }}
          >
            <motion.img
              src={src}
              alt={alt}
              draggable={false}
              className="max-h-full max-w-full object-contain"
              {...dialog}
              onMouseDown={(e) => e.stopPropagation()}
            />
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>,
    document.body,
  );
}

/** Theme-independent button for the dark scrim (panel tokens would blend in). */
function ToolbarButton({
  onClick,
  icon,
  children,
  autoFocus = false,
}: {
  onClick: () => void;
  icon: "download" | "x";
  children: ReactNode;
  /** Focus target of the dialog when it opens. */
  autoFocus?: boolean;
}) {
  return (
    <button
      type="button"
      data-autofocus={autoFocus ? "" : undefined}
      className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg bg-white/10 px-3 text-sm font-medium text-white transition-colors hover:bg-white/20 focus-visible:outline-2 focus-visible:outline-white"
      onClick={onClick}
    >
      <Icon name={icon} />
      {children}
    </button>
  );
}
