// Generated 64×64 skin textures (PNG data URLs) for the mock skin library.
// Tiny uncompressed PNG encoder: mock-only, a few KB of work per skin.
import type { Cape, Skin, SkinModel } from "../bindings";

type Rgb = [number, number, number];

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of bytes) c = (CRC_TABLE[(c ^ b) & 0xff] ?? 0) ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function u32(n: number): number[] {
  return [(n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
}

function chunk(type: string, data: Uint8Array): number[] {
  const body = new Uint8Array(4 + data.length);
  for (let i = 0; i < 4; i += 1) body[i] = type.charCodeAt(i);
  body.set(data, 4);
  return [...u32(data.length), ...body, ...u32(crc32(body))];
}

/** Encodes RGBA pixels as a PNG (stored deflate blocks, no compression). */
export function encodePng(width: number, height: number, rgba: Uint8Array): Uint8Array {
  const raw = new Uint8Array(height * (1 + width * 4));
  for (let y = 0; y < height; y += 1) {
    raw[y * (1 + width * 4)] = 0;
    raw.set(rgba.subarray(y * width * 4, (y + 1) * width * 4), y * (1 + width * 4) + 1);
  }
  const blocks: number[] = [0x78, 0x01];
  for (let off = 0; off < raw.length || off === 0; off += 65535) {
    const len = Math.min(65535, raw.length - off);
    const last = off + len >= raw.length ? 1 : 0;
    blocks.push(last, len & 0xff, len >>> 8, ~len & 0xff, (~len >>> 8) & 0xff);
    for (let i = 0; i < len; i += 1) blocks.push(raw[off + i] ?? 0);
    if (last) break;
  }
  let a = 1;
  let b = 0;
  for (const byte of raw) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  blocks.push(...u32(((b << 16) | a) >>> 0));
  const ihdr = new Uint8Array([...u32(width), ...u32(height), 8, 6, 0, 0, 0]);
  return new Uint8Array([
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    ...chunk("IHDR", ihdr),
    ...chunk("IDAT", new Uint8Array(blocks)),
    ...chunk("IEND", new Uint8Array()),
  ]);
}

export function pngDataUrl(png: Uint8Array): string {
  let bin = "";
  for (const b of png) bin += String.fromCharCode(b);
  return `data:image/png;base64,${btoa(bin)}`;
}

export interface SkinPalette {
  skin: Rgb;
  hair: Rgb;
  shirt: Rgb;
  pants: Rgb;
  shoes: Rgb;
  eyes: Rgb;
}

class Canvas {
  readonly px: Uint8Array;
  constructor(
    readonly w: number,
    readonly h: number,
  ) {
    this.px = new Uint8Array(w * h * 4);
  }
  rect(x: number, y: number, w: number, h: number, [r, g, b]: Rgb, shade = 0) {
    for (let yy = y; yy < y + h; yy += 1)
      for (let xx = x; xx < x + w; xx += 1) {
        const i = (yy * this.w + xx) * 4;
        // Subtle per-pixel noise so the texture does not look flat.
        const n = ((xx * 7 + yy * 13) % 5) - 2 + shade;
        this.px[i] = Math.max(0, Math.min(255, r + n * 4));
        this.px[i + 1] = Math.max(0, Math.min(255, g + n * 4));
        this.px[i + 2] = Math.max(0, Math.min(255, b + n * 4));
        this.px[i + 3] = 255;
      }
  }
}

/** Paints a simple humanoid on the standard 64×64 skin layout. */
export function paintSkin(p: SkinPalette, model: SkinModel): Uint8Array {
  const c = new Canvas(64, 64);
  const arm = model === "slim" ? 14 : 16;
  // Head (all faces) + hair on top and the upper rows of each side.
  c.rect(0, 8, 32, 8, p.skin);
  c.rect(8, 0, 16, 8, p.hair);
  c.rect(0, 8, 32, 2, p.hair, -1);
  c.rect(24, 8, 8, 6, p.hair, -1);
  c.rect(9, 12, 2, 1, [250, 250, 250]);
  c.rect(13, 12, 2, 1, [250, 250, 250]);
  c.rect(10, 12, 1, 1, p.eyes);
  c.rect(13, 12, 1, 1, p.eyes);
  c.rect(11, 14, 2, 1, [Math.max(0, p.skin[0] - 60), Math.max(0, p.skin[1] - 60), Math.max(0, p.skin[2] - 60)]);
  // Body.
  c.rect(16, 16, 24, 16, p.shirt);
  // Arms: sleeve on the upper part, skin below.
  c.rect(40, 16, arm, 16, p.skin);
  c.rect(40, 16, arm, 9, p.shirt, 1);
  c.rect(32, 48, arm, 16, p.skin);
  c.rect(32, 48, arm, 9, p.shirt, 1);
  // Legs with shoes.
  c.rect(0, 16, 16, 16, p.pants);
  c.rect(0, 29, 16, 3, p.shoes);
  c.rect(16, 48, 16, 16, p.pants);
  c.rect(16, 61, 16, 3, p.shoes);
  return c.px;
}

export function skinDataUrl(p: SkinPalette, model: SkinModel): string {
  return pngDataUrl(encodePng(64, 64, paintSkin(p, model)));
}

function hash(s: string): number {
  let h = 2166136261;
  for (const ch of s) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return h >>> 0;
}

function hsl(h: number, s: number, l: number): Rgb {
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
  return [Math.round(f(0) * 255), Math.round(f(8) * 255), Math.round(f(4) * 255)];
}

/** Deterministic palette from any seed string (used for `skin_add`). */
export function paletteFromSeed(seed: string): SkinPalette {
  const h = hash(seed);
  const tones: Rgb[] = [
    [240, 200, 160],
    [198, 140, 100],
    [140, 95, 65],
  ];
  return {
    skin: tones[h % tones.length] ?? [240, 200, 160],
    hair: hsl((h >>> 3) % 360, 0.45, 0.25),
    shirt: hsl((h >>> 7) % 360, 0.6, 0.45),
    pants: hsl((h >>> 11) % 360, 0.35, 0.3),
    shoes: [50, 45, 45],
    eyes: hsl((h >>> 15) % 360, 0.7, 0.35),
  };
}

function capeDataUrl(main: Rgb, accent: Rgb): string {
  const c = new Canvas(64, 32);
  c.rect(0, 0, 22, 17, main);
  c.rect(1, 1, 10, 16, main, 1);
  c.rect(4, 5, 4, 6, accent);
  return pngDataUrl(encodePng(64, 32, c.px));
}

export function seedSkins(): Skin[] {
  const mk = (id: string, name: string, model: SkinModel, p: SkinPalette, localOnly: boolean): Skin => ({
    id,
    name,
    model,
    hash: hash(id).toString(16).padStart(8, "0").repeat(8),
    url: skinDataUrl(p, model),
    localPath: null,
    localOnly,
  });
  return [
    mk("skin-explorer", "Explorer", "classic", { skin: [240, 200, 160], hair: [70, 45, 25], shirt: [40, 160, 170], pants: [60, 60, 140], shoes: [60, 60, 60], eyes: [60, 50, 160] }, false),
    mk("skin-ranger", "Ranger", "slim", { skin: [235, 190, 150], hair: [200, 90, 30], shirt: [70, 130, 60], pants: [110, 80, 50], shoes: [60, 40, 30], eyes: [40, 120, 40] }, false),
    mk("skin-knight", "Knight", "classic", { skin: [198, 140, 100], hair: [150, 150, 160], shirt: [170, 170, 185], pants: [120, 120, 135], shoes: [80, 80, 90], eyes: [30, 30, 30] }, true),
    mk("skin-ninja", "Ninja", "slim", { skin: [140, 95, 65], hair: [25, 25, 30], shirt: [35, 35, 45], pants: [30, 30, 40], shoes: [150, 30, 30], eyes: [200, 40, 40] }, true),
  ];
}

export function seedCapes(): Cape[] {
  return [
    { id: "cape-founder", name: "MineUP Founder", url: capeDataUrl([30, 120, 90], [240, 220, 120]), active: false },
    { id: "cape-builder", name: "Builder", url: capeDataUrl([150, 40, 40], [240, 240, 240]), active: false },
  ];
}
