import { AnimatePresence, type MotionProps, motion } from "motion/react";
import {
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
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
  const [zoom, setZoom] = useState(MIN_ZOOM);
  const stepZoom = useRef<(factor: number) => void>(() => {});

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
            <ToolbarButton
              onClick={() => stepZoom.current(1 / ZOOM_STEP)}
              icon="minus"
              label={t("social.zoomOut")}
              disabled={zoom <= MIN_ZOOM}
            />
            <span className="min-w-12 text-center text-xs text-white/70 tabular-nums">
              {Math.round(zoom * 100)}%
            </span>
            <ToolbarButton
              onClick={() => stepZoom.current(ZOOM_STEP)}
              icon="plus"
              label={t("social.zoomIn")}
              disabled={zoom >= MAX_ZOOM}
            />
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
            className="flex min-h-0 flex-1 items-center justify-center overflow-hidden p-3"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) onClose();
            }}
          >
            <Zoomable
              src={src}
              alt={alt}
              motionProps={dialog}
              stepRef={stepZoom}
              onZoom={setZoom}
            />
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>,
    document.body,
  );
}

/** Zoom range and step of the wheel / keyboard zoom. */
const MIN_ZOOM = 1;
const MAX_ZOOM = 6;
const ZOOM_STEP = 1.25;

const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

/**
 * The photo itself: fits the screen at 1x, zooms with the wheel (anchored at the
 * cursor), the +/− buttons or double click, and is dragged around with the mouse
 * while zoomed. Keyboard: +/- and arrows.
 */
function Zoomable({
  src,
  alt,
  motionProps,
  stepRef,
  onZoom,
}: {
  src: string;
  alt: string;
  motionProps: MotionProps;
  /** Toolbar zoom: multiplies the current zoom by the factor. */
  stepRef?: RefObject<((factor: number) => void) | null>;
  onZoom?: (zoom: number) => void;
}) {
  const [zoom, setZoomState] = useState(MIN_ZOOM);
  const setZoom = (z: number) => {
    setZoomState(z);
    onZoom?.(z);
  };
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const box = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;

  // The toolbar reads the live zoom through this handle.
  useEffect(() => {
    if (stepRef) stepRef.current = (factor) => zoomAt(zoomRef.current * factor);
  });

  // A new picture always opens fitted.
  useEffect(() => {
    const id = setTimeout(() => {
      setZoom(MIN_ZOOM);
      setOffset({ x: 0, y: 0 });
    });
    return () => clearTimeout(id);
  }, [src]);

  const zoomAt = (next: number, originX = 0.5, originY = 0.5) => {
    const z = clampZoom(next);
    const rect = box.current?.getBoundingClientRect();
    // Anchor the point under the cursor: scale the offset by the zoom change.
    if (rect && z > MIN_ZOOM) {
      const k = z / zoom;
      setOffset((o) => ({
        x: o.x + (0.5 - originX) * rect.width * (k - 1),
        y: o.y + (0.5 - originY) * rect.height * (k - 1),
      }));
    } else if (z === MIN_ZOOM) {
      setOffset({ x: 0, y: 0 });
    }
    setZoom(z);
  };

  const onWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const rect = box.current?.getBoundingClientRect();
    const ox = rect ? (e.clientX - rect.left) / rect.width : 0.5;
    const oy = rect ? (e.clientY - rect.top) / rect.height : 0.5;
    zoomAt(e.deltaY < 0 ? zoom * ZOOM_STEP : zoom / ZOOM_STEP, ox, oy);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "+" || e.key === "=") zoomAt(zoom * ZOOM_STEP);
    else if (e.key === "-") zoomAt(zoom / ZOOM_STEP);
    else if (e.key === "0") zoomAt(MIN_ZOOM);
    else if (zoom > MIN_ZOOM && e.key.startsWith("Arrow")) {
      const step = 40;
      const dx = e.key === "ArrowLeft" ? step : e.key === "ArrowRight" ? -step : 0;
      const dy = e.key === "ArrowUp" ? step : e.key === "ArrowDown" ? -step : 0;
      setOffset((o) => ({ x: o.x + dx, y: o.y + dy }));
    } else return;
    e.preventDefault();
  };

  const startDrag = (e: ReactPointerEvent) => {
    if (zoom <= MIN_ZOOM || e.button !== 0) return;
    e.stopPropagation();
    drag.current = { x: e.clientX, y: e.clientY, ox: offset.x, oy: offset.y };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  };
  const moveDrag = (e: ReactPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    setOffset({ x: d.ox + e.clientX - d.x, y: d.oy + e.clientY - d.y });
  };
  const endDrag = () => {
    drag.current = null;
  };

  return (
    <div
      ref={box}
      data-testid="photo-canvas"
      role="group"
      aria-label={alt}
      tabIndex={0}
      onWheel={onWheel}
      onKeyDown={onKey}
      onDoubleClick={() => zoomAt(zoom > MIN_ZOOM ? MIN_ZOOM : 2.5)}
      onPointerDown={startDrag}
      onPointerMove={moveDrag}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      className={`flex size-full items-center justify-center outline-none ${zoom > MIN_ZOOM ? "cursor-grab active:cursor-grabbing" : "cursor-zoom-in"}`}
    >
      <motion.img
        src={src}
        alt={alt}
        draggable={false}
        className="max-h-full max-w-full object-contain select-none"
        style={{ transform: `scale(${zoom}) translate(${offset.x / zoom}px, ${offset.y / zoom}px)` }}
        transition={{ type: "tween", duration: 0.12 }}
        {...motionProps}
        onMouseDown={(e) => e.stopPropagation()}
      />
    </div>
  );
}

/** Zoom buttons shown in the toolbar: −, percentage hint, +. */

/** Theme-independent button for the dark scrim (panel tokens would blend in). */
function ToolbarButton({
  onClick,
  icon,
  children,
  label,
  disabled = false,
  autoFocus = false,
}: {
  onClick: () => void;
  icon: "download" | "x" | "plus" | "minus";
  children?: ReactNode;
  /** Accessible name of an icon-only button. */
  label?: string;
  disabled?: boolean;
  /** Focus target of the dialog when it opens. */
  autoFocus?: boolean;
}) {
  return (
    <button
      type="button"
      data-autofocus={autoFocus ? "" : undefined}
      aria-label={label}
      title={label}
      disabled={disabled}
      className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg bg-white/10 px-3 text-sm font-medium text-white transition-colors hover:bg-white/20 focus-visible:outline-2 focus-visible:outline-white disabled:cursor-default disabled:opacity-40"
      onClick={onClick}
    >
      <Icon name={icon} />
      {children}
    </button>
  );
}