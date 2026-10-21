import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SkinViewer } from "skinview3d";

import type { SkinModel } from "../../bindings";
import { FRONT_H, FRONT_W, drawBlits, frontViewBlits, headBlits, loadSkinImage } from "./skinRender";

/** Static pixel-art render (front body or head) on a Canvas 2D — no WebGL. */
export function SkinFlat({
  url,
  model,
  variant = "body",
  className = "",
}: {
  url: string;
  model: SkinModel;
  variant?: "body" | "head";
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);
  const w = variant === "head" ? 8 : FRONT_W;
  const h = variant === "head" ? 8 : FRONT_H;

  useEffect(() => {
    let alive = true;
    setFailed(false);
    loadSkinImage(url)
      .then((img) => {
        const canvas = ref.current;
        if (!alive || !canvas) return;
        const legacy = img.naturalHeight === 32;
        const ok = drawBlits(canvas, img, variant === "head" ? headBlits() : frontViewBlits(model, legacy));
        if (!ok) setFailed(true);
      })
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, [url, model, variant]);

  return (
    <canvas
      ref={ref}
      width={w}
      height={h}
      aria-hidden
      className={`${className} ${failed ? "rounded-md bg-surface-2" : ""}`}
      style={{ imageRendering: "pixelated", aspectRatio: `${w} / ${h}` }}
    />
  );
}

/**
 * Interactive 3D preview. Exactly one per screen (one WebGL context): the
 * library loads `skinview3d` lazily, only once the canvas is on screen.
 * Falls back to `SkinFlat` when WebGL or the texture fails; a new skin retries.
 */
export function Skin3D({
  url,
  model,
  capeUrl,
  animate,
  className = "",
}: {
  url: string;
  model: SkinModel;
  capeUrl: string | null;
  animate: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewerRef = useRef<SkinViewer | null>(null);
  const [visible, setVisible] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ready, setReady] = useState(false);
  const latest = useRef({ url, model, capeUrl, animate });
  latest.current = { url, model, capeUrl, animate };

  // A different skin gets a fresh attempt after a failure.
  useEffect(() => setFailed(false), [url, model]);

  // Lazy init: wait until the preview box is actually visible.
  useEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        setVisible(true);
        io.disconnect();
      }
    });
    io.observe(el);
    return () => io.disconnect();
  }, []);

  // Viewer lifetime: created when visible and not failed; disposed on failure/unmount.
  useEffect(() => {
    if (!visible || failed) return;
    let disposed = false;
    void (async () => {
      try {
        const sv = await import("skinview3d");
        const canvas = canvasRef.current;
        const box = boxRef.current;
        if (disposed || !canvas || !box) return;
        const cur = latest.current;
        const viewer = new sv.SkinViewer({
          canvas,
          width: box.clientWidth || 240,
          height: box.clientHeight || 320,
          model: cur.model === "slim" ? "slim" : "default",
          zoom: 0.85,
        });
        viewer.autoRotate = cur.animate;
        viewer.autoRotateSpeed = 0.6;
        viewer.animation = cur.animate ? new sv.WalkingAnimation() : null;
        if (viewer.animation) viewer.animation.speed = 0.6;
        if (disposed) {
          viewer.dispose();
          return;
        }
        viewerRef.current = viewer;
        setReady(true);
      } catch {
        if (!disposed) setFailed(true);
      }
    })();
    return () => {
      disposed = true;
      viewerRef.current?.dispose();
      viewerRef.current = null;
      setReady(false);
    };
  }, [visible, failed]);

  // Keep the drawing buffer in sync with the layout.
  useEffect(() => {
    const box = boxRef.current;
    if (!box || !ready || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      const w = box.clientWidth;
      const h = box.clientHeight;
      if (w > 0 && h > 0) viewerRef.current?.setSize(w, h);
    });
    ro.observe(box);
    return () => ro.disconnect();
  }, [ready]);

  // Texture / settings updates reuse the same viewer (and context).
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !ready) return;
    viewer.loadSkin(url, { model: model === "slim" ? "slim" : "default" }).catch(() => setFailed(true));
  }, [url, model, ready]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !ready) return;
    if (capeUrl) viewer.loadCape(capeUrl).catch(() => viewer.loadCape(null));
    else viewer.loadCape(null);
  }, [capeUrl, ready]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !ready) return;
    viewer.autoRotate = animate;
    if (animate && !viewer.animation) {
      void import("skinview3d").then((sv) => {
        if (viewerRef.current === viewer) viewer.animation = new sv.WalkingAnimation();
      });
    } else if (!animate) viewer.animation = null;
  }, [animate, ready]);

  // Pause rendering while the window is hidden.
  useEffect(() => {
    const onVis = () => {
      if (viewerRef.current) viewerRef.current.renderPaused = document.hidden;
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, []);

  return (
    <div ref={boxRef} className={`flex size-full items-center justify-center ${className}`}>
      {failed ? (
        <SkinFlat url={url} model={model} className="h-full" />
      ) : (
        <canvas ref={canvasRef} role="img" aria-label={t("skins.preview3d")} className="size-full cursor-grab active:cursor-grabbing" />
      )}
    </div>
  );
}