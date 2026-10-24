// MineUP app icon generator: a chunky pixel-art "D" built from grass/dirt blocks.
// Original artwork (procedural, no third-party textures). Pure Node, no dependencies.
//
// Usage (from apps/desktop):
//   node src-tauri/icons/source/gen-icon.mjs master     # icons/source/icon-1024.png + public/logo.svg
//   pnpm tauri icon src-tauri/icons/source/icon-1024.png
//   node src-tauri/icons/source/gen-icon.mjs finalize   # crisp integer-scaled PNGs + icon.ico
//
// `tauri icon` resamples with a smoothing filter; `finalize` overwrites the sizes that are
// integer multiples of the pixel grids with nearest-neighbor renders and rebuilds icon.ico,
// using a hand-tuned 16x16 grid for the smallest sizes.

import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const iconsDir = join(here, "..");
const publicDir = join(here, "..", "..", "..", "public");

// Letter "D" in block cells (6 x 7).
const LETTER = ["#####.", "######", "##..##", "##..##", "##..##", "######", "#####."];

const OUTLINE = [0x1b, 0x14, 0x10, 255];
const GRASS = [
  [0x4c, 0x8a, 0x2a, 255],
  [0x5d, 0x9e, 0x33, 255],
  [0x6f, 0xb5, 0x3c, 255],
  [0x86, 0xc9, 0x4a, 255],
];
const DIRT = [
  [0x6a, 0x44, 0x28, 255],
  [0x79, 0x50, 0x2f, 255],
  [0x8a, 0x5a, 0x3b, 255],
  [0x9b, 0x6a, 0x46, 255],
];

// Deterministic PRNG (mulberry32) so the artwork is reproducible.
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Renders the letter into a size x size RGBA grid; `cell` = texture pixels per block. */
function renderGrid(size, cell, seed) {
  const rand = rng(seed);
  const w = LETTER[0].length * cell;
  const h = LETTER.length * cell;
  const ox = Math.floor((size - w) / 2);
  const oy = Math.floor((size - h) / 2);
  const px = Array.from({ length: size * size }, () => null);
  const filled = (cx, cy) => cy >= 0 && cy < LETTER.length && LETTER[cy][cx] === "#";

  for (let cy = 0; cy < LETTER.length; cy++) {
    for (let cx = 0; cx < LETTER[0].length; cx++) {
      if (!filled(cx, cy)) continue;
      const grassy = !filled(cx, cy - 1);
      // Grass depth per column inside the block: irregular "drips" like a grass block side.
      const depth = [];
      for (let x = 0; x < cell; x++) {
        if (cell <= 2) depth.push(1);
        else {
          const r = rand();
          depth.push(r < 0.3 ? 1 : r < 0.8 ? 2 : 3);
        }
      }
      for (let y = 0; y < cell; y++) {
        for (let x = 0; x < cell; x++) {
          const r = rand();
          let c;
          if (grassy && y < depth[x]) {
            c = y === 0 ? (r < 0.5 ? GRASS[3] : GRASS[2]) : r < 0.5 ? GRASS[1] : GRASS[0];
          } else {
            const bottom = y === cell - 1 && cell > 2;
            const i = bottom ? (r < 0.6 ? 0 : 1) : r < 0.15 ? 0 : r < 0.5 ? 1 : r < 0.85 ? 2 : 3;
            c = DIRT[i];
          }
          px[(oy + cy * cell + y) * size + ox + cx * cell + x] = c;
        }
      }
    }
  }
  // 1px dark outline around the letter (8-neighborhood), inside and outside.
  const out = px.slice();
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (px[y * size + x]) continue;
      let near = false;
      for (let dy = -1; dy <= 1 && !near; dy++)
        for (let dx = -1; dx <= 1 && !near; dx++) {
          const nx = x + dx;
          const ny = y + dy;
          if (nx >= 0 && ny >= 0 && nx < size && ny < size && px[ny * size + nx]) near = true;
        }
      if (near) out[y * size + x] = OUTLINE;
    }
  }
  return { size, px: out };
}

// 32x32 grid: 4px blocks. 16x16 grid: 2px blocks (hand-tuned small size).
const GRID32 = renderGrid(32, 4, 0x4d696e65);
const GRID16 = renderGrid(16, 2, 0x55502121);

function crcTable() {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
}
const CRC = crcTable();
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}

/** Nearest-neighbor upscale of a grid to `out` pixels, encoded as RGBA PNG. */
function png(grid, out) {
  const scale = out / grid.size;
  if (!Number.isInteger(scale)) throw new Error(`non-integer scale ${out}/${grid.size}`);
  const raw = Buffer.alloc((out * 4 + 1) * out);
  for (let y = 0; y < out; y++) {
    const row = y * (out * 4 + 1);
    raw[row] = 0;
    for (let x = 0; x < out; x++) {
      const c = grid.px[Math.floor(y / scale) * grid.size + Math.floor(x / scale)];
      if (c) raw.set(c, row + 1 + x * 4);
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(out, 0);
  ihdr.writeUInt32BE(out, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** ICO with PNG-compressed entries. */
function ico(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = [];
  let offset = 6 + 16 * entries.length;
  for (const { size, data } of entries) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    dir.push(e);
  }
  return Buffer.concat([header, ...dir, ...entries.map((e) => e.data)]);
}

const hex = (c) => "#" + c.slice(0, 3).map((v) => v.toString(16).padStart(2, "0")).join("");

/** SVG of the 32x32 grid, horizontal runs merged into rects. */
function svg(grid) {
  const rects = [];
  for (let y = 0; y < grid.size; y++) {
    let x = 0;
    while (x < grid.size) {
      const c = grid.px[y * grid.size + x];
      if (!c) {
        x++;
        continue;
      }
      let end = x + 1;
      while (end < grid.size && grid.px[y * grid.size + end] === c) end++;
      rects.push(`<rect x="${x}" y="${y}" width="${end - x}" height="1" fill="${hex(c)}"/>`);
      x = end;
    }
  }
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${grid.size} ${grid.size}" ` +
    `shape-rendering="crispEdges">\n${rects.join("\n")}\n</svg>\n`
  );
}

const mode = process.argv[2];
if (mode === "master") {
  mkdirSync(here, { recursive: true });
  writeFileSync(join(here, "icon-1024.png"), png(GRID32, 1024));
  mkdirSync(publicDir, { recursive: true });
  writeFileSync(join(publicDir, "logo.svg"), svg(GRID32));
} else if (mode === "finalize") {
  const files = {
    "32x32.png": png(GRID32, 32),
    "64x64.png": png(GRID32, 64),
    "128x128.png": png(GRID32, 128),
    "128x128@2x.png": png(GRID32, 256),
    "icon.png": png(GRID32, 512),
  };
  for (const [name, data] of Object.entries(files)) writeFileSync(join(iconsDir, name), data);
  const entries = [
    { size: 16, data: png(GRID16, 16) },
    { size: 32, data: png(GRID32, 32) },
    { size: 48, data: png(GRID16, 48) },
    { size: 64, data: png(GRID32, 64) },
    { size: 128, data: png(GRID32, 128) },
    { size: 256, data: png(GRID32, 256) },
  ];
  writeFileSync(join(iconsDir, "icon.ico"), ico(entries));
} else {
  console.error("usage: gen-icon.mjs master|finalize");
  process.exit(2);
}
