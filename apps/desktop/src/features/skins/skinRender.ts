// Flat 2D front view of a Minecraft skin (thumbnails and light mode).
// Canvas 2D only — no WebGL context is created here.
import type { SkinModel } from "../../bindings";

export interface Blit {
  /** Source rect on the skin texture. */
  sx: number;
  sy: number;
  w: number;
  h: number;
  /** Destination on a 16×32 front-view canvas. */
  dx: number;
  dy: number;
  /** Draw mirrored horizontally (legacy 64×32 skins reuse the right limbs). */
  flip?: boolean;
}

export const FRONT_W = 16;
export const FRONT_H = 32;

/**
 * Rects that compose the front view: base layer first, then the overlay.
 * `legacy` = 64×32 texture without separate left limbs or body/limb overlays.
 */
export function frontViewBlits(model: SkinModel, legacy: boolean): Blit[] {
  const arm = model === "slim" ? 3 : 4;
  const rightArmX = 4 - arm;
  const base: Blit[] = [
    { sx: 8, sy: 8, w: 8, h: 8, dx: 4, dy: 0 }, // head
    { sx: 20, sy: 20, w: 8, h: 12, dx: 4, dy: 8 }, // body
    { sx: 44, sy: 20, w: arm, h: 12, dx: rightArmX, dy: 8 }, // right arm (viewer's left)
    { sx: 4, sy: 20, w: 4, h: 12, dx: 4, dy: 20 }, // right leg
    legacy
      ? { sx: 44, sy: 20, w: arm, h: 12, dx: 12, dy: 8, flip: true }
      : { sx: 36, sy: 52, w: arm, h: 12, dx: 12, dy: 8 }, // left arm
    legacy
      ? { sx: 4, sy: 20, w: 4, h: 12, dx: 8, dy: 20, flip: true }
      : { sx: 20, sy: 52, w: 4, h: 12, dx: 8, dy: 20 }, // left leg
  ];
  const overlay: Blit[] = [{ sx: 40, sy: 8, w: 8, h: 8, dx: 4, dy: 0 }]; // hat
  if (!legacy) {
    overlay.push(
      { sx: 20, sy: 36, w: 8, h: 12, dx: 4, dy: 8 }, // jacket
      { sx: 44, sy: 36, w: arm, h: 12, dx: rightArmX, dy: 8 },
      { sx: 52, sy: 52, w: arm, h: 12, dx: 12, dy: 8 },
      { sx: 4, sy: 36, w: 4, h: 12, dx: 4, dy: 20 },
      { sx: 4, sy: 52, w: 4, h: 12, dx: 8, dy: 20 },
    );
  }
  return [...base, ...overlay];
}

/** Head-only blits (face + hat) on an 8×8 canvas. */
export function headBlits(): Blit[] {
  return [
    { sx: 8, sy: 8, w: 8, h: 8, dx: 0, dy: 0 },
    { sx: 40, sy: 8, w: 8, h: 8, dx: 0, dy: 0 },
  ];
}

const images = new Map<string, Promise<HTMLImageElement>>();

/** Loads (and caches) a skin image. */
export function loadSkinImage(url: string): Promise<HTMLImageElement> {
  let p = images.get(url);
  if (!p) {
    p = new Promise((resolve, reject) => {
      const img = new Image();
      img.crossOrigin = "anonymous";
      img.onload = () => resolve(img);
      img.onerror = () => reject(new Error("skin image failed to load"));
      img.src = url;
    });
    images.set(url, p);
    // Do not cache failures.
    p.catch(() => images.delete(url));
  }
  return p;
}

/** Draws blits onto `canvas` (sized by the caller in texture pixels). */
export function drawBlits(canvas: HTMLCanvasElement, img: HTMLImageElement, blits: Blit[]): boolean {
  const ctx = canvas.getContext("2d");
  if (!ctx) return false;
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  for (const b of blits) {
    if (b.flip) {
      ctx.save();
      ctx.translate(b.dx + b.w, b.dy);
      ctx.scale(-1, 1);
      ctx.drawImage(img, b.sx, b.sy, b.w, b.h, 0, 0, b.w, b.h);
      ctx.restore();
    } else {
      ctx.drawImage(img, b.sx, b.sy, b.w, b.h, b.dx, b.dy, b.w, b.h);
    }
  }
  return true;
}
