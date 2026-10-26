// Built-in voxel-style backgrounds: original artwork drawn from a few numbers
// (blocky step paths and pixel rects, no images). Layers: sky (static),
// clouds (drift), far hills (sway) and the foreground (static). Animations are
// CSS transforms on HTML layers only, so they stay on the compositor.
import { type ReactNode, useId, useMemo } from "react";

import type { BackgroundId } from "../../lib/backdrop";

const W = 320;
const H = 180;
const VIEW = `0 0 ${W} ${H}`;

interface Hill {
  color: string;
  /** Top band colour (grass, snow...). */
  cap?: string;
  capH?: number;
  /** Mean height from the bottom (or from the top for ceilings). */
  base: number;
  amp: number;
  /** Block size in view units. */
  step: number;
  seed: number;
  ceiling?: boolean;
}

interface Scene {
  sky: string[];
  orb?: { x: number; y: number; s: number; color: string; glow: string };
  stars?: string;
  clouds?: string;
  far: Hill[];
  near: Hill[];
  /** Foreground details drawn over the near hills (trees, ores...). */
  extra?: (near: number[][]) => ReactNode;
}

/** Deterministic block heights (two sines, quantised to the block grid). */
function heights(h: Hill): number[] {
  const out: number[] = [];
  for (let x = 0; x <= W; x += h.step) {
    const n = 0.6 * Math.sin(x * 0.021 + h.seed) + 0.4 * Math.sin(x * 0.057 + h.seed * 1.7);
    out.push(Math.round((h.base + h.amp * n) / h.step) * h.step);
  }
  return out;
}

/** Closed step polygon for column heights `hs` (+`extra` thickness offset). */
function stepPath(hs: number[], step: number, ceiling: boolean, offset = 0): string {
  const y = (v: number) => (ceiling ? v - offset : H - v + offset);
  let d = `M0 ${ceiling ? 0 : H}`;
  hs.forEach((v, i) => {
    d += `V${y(v)}H${Math.min(W, (i + 1) * step)}`;
  });
  return `${d}V${ceiling ? 0 : H}Z`;
}

function HillShape({ hill, hs }: { hill: Hill; hs: number[] }) {
  const ceiling = hill.ceiling ?? false;
  const capH = hill.capH ?? hill.step;
  return hill.cap ? (
    <>
      <path d={stepPath(hs, hill.step, ceiling)} fill={hill.cap} />
      <path d={stepPath(hs, hill.step, ceiling, capH)} fill={hill.color} />
    </>
  ) : (
    <path d={stepPath(hs, hill.step, ceiling)} fill={hill.color} />
  );
}

/** Tiny deterministic PRNG (mulberry32) for star / ore placement. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Surface y (from the top) of column `x` of a near layer. */
const topAt = (hs: number[], step: number, x: number) => H - (hs[Math.floor(x / step)] ?? 0);

function Trees({ hs, step, trunk, leaves, every, seed }: { hs: number[]; step: number; trunk: string; leaves: string[]; every: number; seed: number }) {
  const r = rng(seed);
  const out: ReactNode[] = [];
  for (let x = 10; x < W - 10; x += every + Math.floor(r() * every)) {
    const gx = Math.floor(x / 4) * 4;
    const y = topAt(hs, step, gx);
    const tall = 12 + Math.floor(r() * 3) * 4;
    out.push(
      <g key={x}>
        <rect x={gx} y={y - tall} width={4} height={tall} fill={trunk} />
        <rect x={gx - 8} y={y - tall - 8} width={20} height={12} fill={leaves[0]} />
        <rect x={gx - 4} y={y - tall - 14} width={12} height={6} fill={leaves[1] ?? leaves[0]} />
      </g>,
    );
  }
  return <>{out}</>;
}

function Clouds({ color }: { color: string }) {
  // Kept away from the left/right edges so the two drifting copies join seamlessly.
  const shapes: Array<[number, number, number]> = [
    [40, 26, 36],
    [120, 16, 48],
    [196, 34, 28],
    [252, 20, 40],
  ];
  return (
    <g fill={color}>
      {shapes.map(([x, y, w]) => (
        <g key={x}>
          <rect x={x} y={y} width={w} height={8} />
          <rect x={x + 8} y={y - 4} width={w - 16} height={4} />
        </g>
      ))}
    </g>
  );
}

const SCENES: Record<BackgroundId, Scene> = {
  plains: {
    sky: ["#f9a66c", "#f7c59f", "#9fd3ef"],
    orb: { x: 236, y: 52, s: 20, color: "#ffe9a8", glow: "#ffd27a" },
    clouds: "#fff4e6",
    far: [{ color: "#6f9f8f", base: 70, amp: 14, step: 8, seed: 1 }],
    near: [
      { color: "#7a5532", cap: "#5fae3c", capH: 6, base: 44, amp: 10, step: 8, seed: 4 },
    ],
  },
  forest: {
    sky: ["#0b1330", "#1a2a55", "#2c4068"],
    orb: { x: 288, y: 36, s: 16, color: "#e8ecf5", glow: "#9fb4e0" },
    stars: "#dfe6ff",
    far: [{ color: "#1d3346", base: 74, amp: 12, step: 8, seed: 2 }],
    near: [{ color: "#2a1d14", cap: "#1f4a2a", capH: 4, base: 40, amp: 8, step: 8, seed: 7 }],
    extra: ([hs = []]) => <Trees hs={hs} step={8} trunk="#3b2a1c" leaves={["#173a22", "#1f4d2c"]} every={22} seed={11} />,
  },
  nether: {
    sky: ["#2a0606", "#5a0f0c", "#a3260f"],
    far: [
      { color: "#3d0a0a", base: 46, amp: 12, step: 8, seed: 3, ceiling: true },
      { color: "#5b1410", base: 64, amp: 16, step: 8, seed: 5 },
    ],
    near: [
      { color: "#6e1712", cap: "#8a1f17", capH: 4, base: 34, amp: 18, step: 8, seed: 9 },
      // Lava lake in front.
      { color: "#ff7a1a", cap: "#ffb347", capH: 2, base: 22, amp: 0, step: 8, seed: 0 },
    ],
    extra: () => {
      const r = rng(5);
      return (
        <g fill="#ffd36b">
          {Array.from({ length: 7 }, (_, i) => (
            <rect key={i} x={Math.floor(r() * 76) * 4} y={4 + Math.floor(r() * 6) * 4} width={4} height={4} opacity={0.9} />
          ))}
        </g>
      );
    },
  },
  ocean: {
    sky: ["#5fb7f2", "#a7dcf7", "#d8f1fb"],
    orb: { x: 284, y: 36, s: 18, color: "#fff6c7", glow: "#ffe58a" },
    clouds: "#ffffff",
    far: [{ color: "#7fb08a", cap: "#e9d9a1", capH: 3, base: 66, amp: 6, step: 8, seed: 6 }],
    near: [
      { color: "#2d7fc1", base: 62, amp: 0, step: 8, seed: 0 },
      { color: "#2369a6", base: 40, amp: 0, step: 8, seed: 0 },
      { color: "#1c548a", base: 18, amp: 0, step: 8, seed: 0 },
    ],
    extra: () => (
      <g fill="#bfe6ff" opacity={0.7}>
        {[30, 90, 150, 210, 270].map((x, i) => (
          <rect key={x} x={x} y={124 + (i % 2) * 22} width={16} height={2} />
        ))}
      </g>
    ),
  },
  snowy: {
    sky: ["#7d9cc4", "#b8cbe2", "#e4edf6"],
    clouds: "#f4f8fc",
    far: [{ color: "#8697ad", cap: "#f2f6fb", capH: 12, base: 104, amp: 34, step: 8, seed: 8 }],
    near: [{ color: "#5f6c7c", cap: "#ffffff", capH: 8, base: 50, amp: 16, step: 8, seed: 12 }],
    extra: ([hs = []]) => <Trees hs={hs} step={8} trunk="#3f3226" leaves={["#2f5546", "#e9f1f7"]} every={34} seed={3} />,
  },
  cave: {
    sky: ["#16181c", "#22252b", "#2b2f36"],
    far: [
      { color: "#33373e", base: 40, amp: 12, step: 8, seed: 4, ceiling: true },
      { color: "#3a3f47", base: 52, amp: 10, step: 8, seed: 10 },
    ],
    near: [
      { color: "#4b5059", base: 30, amp: 14, step: 8, seed: 13, ceiling: true },
      { color: "#4f545d", cap: "#5d636d", capH: 4, base: 34, amp: 12, step: 8, seed: 2 },
    ],
    extra: () => {
      const r = rng(21);
      const ores = ["#5ee0e6", "#f2c94c", "#e24b4b", "#4cd07d"];
      const cluster = (x: number, y: number, c: string, k: number) => (
        <g key={k} fill={c}>
          <rect x={x} y={y} width={4} height={4} />
          <rect x={x + 6} y={y + 2} width={2} height={2} />
          <rect x={x + 2} y={y + 6} width={4} height={2} />
        </g>
      );
      return (
        <>
          <radialGradient id="torch" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="#ffb347" stopOpacity="0.35" />
            <stop offset="1" stopColor="#ffb347" stopOpacity="0" />
          </radialGradient>
          <rect x={120} y={60} width={80} height={80} fill="url(#torch)" />
          <rect x={158} y={96} width={2} height={8} fill="#7a5532" />
          <rect x={157} y={92} width={4} height={4} fill="#ffd36b" />
          {Array.from({ length: 10 }, (_, i) =>
            cluster(Math.floor(r() * 76) * 4, i % 2 ? 150 + Math.floor(r() * 6) * 4 : 6 + Math.floor(r() * 4) * 4, ores[i % 4] ?? "#fff", i),
          )}
        </>
      );
    },
  },
  desert: {
    sky: ["#3b2a5c", "#c0607a", "#f5a25d"],
    orb: { x: 160, y: 104, s: 26, color: "#ffd27a", glow: "#ff9f5a" },
    stars: "#f6e7ff",
    far: [{ color: "#c98a52", base: 60, amp: 8, step: 8, seed: 14 }],
    near: [{ color: "#d9a35f", cap: "#ecc27c", capH: 4, base: 36, amp: 8, step: 8, seed: 15 }],
    extra: ([hs = []]) => (
      <g fill="#3f7a3a">
        {[48, 212, 276].map((x) => {
          const y = topAt(hs, 8, x);
          return (
            <g key={x}>
              <rect x={x} y={y - 20} width={4} height={20} />
              <rect x={x - 4} y={y - 14} width={4} height={6} />
              <rect x={x + 4} y={y - 16} width={4} height={6} />
            </g>
          );
        })}
      </g>
    ),
  },
  cherry: {
    sky: ["#9ec9f0", "#d8e6f6", "#fbe3ee"],
    orb: { x: 252, y: 40, s: 16, color: "#fff7d6", glow: "#ffe9b0" },
    clouds: "#ffffff",
    far: [{ color: "#9cb98d", base: 66, amp: 12, step: 8, seed: 16 }],
    near: [{ color: "#6b4a33", cap: "#79b85a", capH: 5, base: 42, amp: 9, step: 8, seed: 17 }],
    extra: ([hs = []]) => <Trees hs={hs} step={8} trunk="#5a3b2e" leaves={["#f4a6c6", "#f9c8dc"]} every={30} seed={8} />,
  },
};

function Stars({ color, seed }: { color: string; seed: number }) {
  const r = rng(seed);
  return (
    <g fill={color}>
      {Array.from({ length: 36 }, (_, i) => (
        <rect key={i} x={Math.floor(r() * 160) * 2} y={Math.floor(r() * 40) * 2} width={i % 5 ? 1 : 2} height={i % 5 ? 1 : 2} opacity={0.5 + r() * 0.5} />
      ))}
    </g>
  );
}

/** Blocky disc (three stacked bands) so the sun/moon does not read as a flat square. */
function Orb({ x, y, s, color }: { x: number; y: number; s: number; color: string }) {
  const q = s / 4;
  return (
    <g fill={color}>
      <rect x={x - s / 2} y={y - q} width={s} height={2 * q} />
      <rect x={x - s / 2 + q / 2} y={y - s / 2 + q / 2} width={s - q} height={s - q} />
      <rect x={x - q} y={y - s / 2} width={2 * q} height={s} />
    </g>
  );
}

const svgProps = {
  viewBox: VIEW,
  preserveAspectRatio: "xMidYMax slice",
  shapeRendering: "crispEdges",
  className: "absolute inset-0 size-full",
} as const;

/**
 * Renders a background scene filling its (positioned) parent. `animate`
 * enables cloud drift and hill sway (the CSS also disables them in light
 * mode and with reduced motion).
 */
export function BackgroundScene({ id, animate = false, className = "" }: { id: BackgroundId; animate?: boolean; className?: string }) {
  const uid = useId().replace(/:/g, "");
  const scene = SCENES[id];
  const far = useMemo(() => scene.far.map(heights), [scene]);
  const near = useMemo(() => scene.near.map(heights), [scene]);
  const sky = `sky-${uid}`;
  const glow = `glow-${uid}`;
  return (
    <div aria-hidden className={`pointer-events-none overflow-hidden ${className}`} data-background={id}>
      <svg {...svgProps}>
        <defs>
          <linearGradient id={sky} x1="0" y1="0" x2="0" y2="1">
            {scene.sky.map((c, i) => (
              <stop key={c} offset={i / Math.max(1, scene.sky.length - 1)} stopColor={c} />
            ))}
          </linearGradient>
          {scene.orb ? (
            <radialGradient id={glow} cx="0.5" cy="0.5" r="0.5">
              <stop offset="0" stopColor={scene.orb.glow} stopOpacity="0.55" />
              <stop offset="1" stopColor={scene.orb.glow} stopOpacity="0" />
            </radialGradient>
          ) : null}
        </defs>
        <rect width={W} height={H} fill={`url(#${sky})`} />
        {scene.stars ? <Stars color={scene.stars} seed={id.length * 7} /> : null}
        {scene.orb ? (
          <>
            <rect x={scene.orb.x - scene.orb.s * 2} y={scene.orb.y - scene.orb.s * 2} width={scene.orb.s * 4} height={scene.orb.s * 4} fill={`url(#${glow})`} />
            <Orb {...scene.orb} />
          </>
        ) : null}
      </svg>
      {scene.clouds ? (
        <div className={`absolute inset-y-0 left-0 flex w-[200%] ${animate ? "bg-drift" : ""}`}>
          {[0, 1].map((k) => (
            <div key={k} className="relative h-full w-1/2">
              <svg {...svgProps} opacity={0.85}>
                <Clouds color={scene.clouds ?? "#fff"} />
              </svg>
            </div>
          ))}
        </div>
      ) : null}
      <div className={`absolute inset-y-0 -inset-x-[2%] ${animate ? "bg-sway" : ""}`}>
        <svg {...svgProps}>
          {scene.far.map((h, i) => (
            <HillShape key={i} hill={h} hs={far[i] ?? []} />
          ))}
        </svg>
      </div>
      <svg {...svgProps}>
        {scene.near.map((h, i) => (
          <HillShape key={i} hill={h} hs={near[i] ?? []} />
        ))}
        {scene.extra?.(near)}
      </svg>
    </div>
  );
}

/** Full-window background behind the shell (fixed, below everything). */
export function AppBackground({ id, animate }: { id: BackgroundId; animate: boolean }) {
  return <BackgroundScene id={id} animate={animate} className="fixed inset-0 -z-10" />;
}
