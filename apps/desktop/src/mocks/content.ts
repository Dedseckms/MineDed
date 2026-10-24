// Mock catalog (Modrinth / CurseForge), installed content and loader versions.
// Project data is source-provided text (not UI strings), so it stays in English.
import type {
  ContentFilter,
  ContentKind,
  ContentSourceKind,
  ContentVersion,
  InstalledContent,
  LoaderKind,
  LoaderVersion,
  ProjectSummary,
  SearchPage,
  SearchQuery,
} from "../bindings";

export interface MockProject extends ProjectSummary {
  loaders: LoaderKind[];
  mc: string[];
  /** Required dependencies (project ids of the same source). */
  requires: string[];
  incompatible: string[];
  /** RFC 3339. */
  updated: string;
  created: string;
  /** CurseForge `allowModDistribution=false`. */
  manual: boolean;
}

const MODERN = ["1.21.10", "1.21.8", "1.21.5", "1.21.4", "1.21.1", "1.20.6", "1.20.4", "1.20.1"];
const LEGACY = ["1.20.1", "1.19.4", "1.19.2", "1.18.2", "1.16.5", "1.12.2"];
const ALL_MC = [...new Set([...MODERN, ...LEGACY])];

const FQ: LoaderKind[] = ["fabric", "quilt"];
const FORGES: LoaderKind[] = ["forge", "neoForge"];
const ANY: LoaderKind[] = ["fabric", "quilt", "forge", "neoForge"];

type Row = [
  id: string,
  slug: string,
  title: string,
  author: string,
  downloads: number,
  loaders: LoaderKind[],
  categories: string,
  description: string,
  extra?: Partial<Pick<MockProject, "requires" | "incompatible" | "mc" | "manual">>,
];

function project(source: ContentSourceKind, kind: ContentKind, row: Row, i: number): MockProject {
  const [projectId, slug, title, author, downloads, loaders, categories, description, extra] = row;
  const day = (n: number) => new Date(Date.UTC(2026, 9, 18) - n * 86_400_000).toISOString();
  return {
    source,
    projectId,
    slug,
    title,
    description,
    author,
    iconUrl: null,
    downloads,
    kind,
    categories: categories.split(","),
    loaders,
    mc: extra?.mc ?? (loaders.some((l) => l === "forge") && !loaders.includes("fabric") ? ALL_MC : MODERN),
    requires: extra?.requires ?? [],
    incompatible: extra?.incompatible ?? [],
    updated: day((i * 7) % 90),
    created: day(200 + i * 37),
    manual: extra?.manual ?? false,
  };
}

const MR_MODS: Row[] = [
  ["P7dR8mSH", "fabric-api", "Fabric API", "modmuss50", 120_400_000, FQ, "library", "Lightweight and modular API providing common hooks and intercompatibility measures utilized by mods using the Fabric toolchain."],
  ["AANobbMI", "sodium", "Sodium", "jellysquid3", 98_200_000, [...FQ, "neoForge"], "optimization", "The fastest and most compatible rendering optimization mod for Minecraft.", { incompatible: ["canvas"] }],
  ["YL57xq9U", "iris", "Iris Shaders", "coderbot", 71_500_000, [...FQ, "neoForge"], "decoration,optimization", "A modern shader pack loader for Minecraft intended to be compatible with existing OptiFine shader packs.", { requires: ["AANobbMI"] }],
  ["gvQqBUqZ", "lithium", "Lithium", "jellysquid3", 54_300_000, [...FQ, "neoForge"], "optimization", "No-compromises game logic optimization mod. Well suited for clients and servers of all kinds."],
  ["uXXizFIs", "ferrite-core", "FerriteCore", "malte0811", 50_100_000, ANY, "optimization", "Memory usage optimizations."],
  ["NNAgCjsB", "entityculling", "Entity Culling", "tr7zw", 41_800_000, ANY, "optimization", "Using async path-tracing to hide Block-/Entities that are not visible."],
  ["mOgUt4GM", "modmenu", "Mod Menu", "Prospector", 39_900_000, FQ, "utility", "Adds a mod menu to view the list of mods you have installed.", { requires: ["P7dR8mSH"] }],
  ["9s6osm5g", "cloth-config", "Cloth Config API", "shedaniel", 37_200_000, ANY, "library", "Configuration Library for Minecraft Mods."],
  ["PtjYWJkn", "sodium-extra", "Sodium Extra", "FlashyReese", 25_600_000, FQ, "optimization", "A Sodium addon that adds features that shouldn't be in Sodium.", { requires: ["AANobbMI", "P7dR8mSH"] }],
  ["Bh37bMuy", "reeses-sodium-options", "Reese's Sodium Options", "FlashyReese", 24_100_000, FQ, "utility", "Alternative Options Menu for Sodium.", { requires: ["AANobbMI"] }],
  ["1IjD5062", "continuity", "Continuity", "PepperCode1", 19_700_000, FQ, "decoration", "A Fabric mod that allows for efficient connected textures."],
  ["w7ThoJFB", "zoomify", "Zoomify", "isXander", 18_300_000, FQ, "utility", "A zoom mod with infinite customizability.", { requires: ["P7dR8mSH", "1eAoo2KR"] }],
  ["1eAoo2KR", "yacl", "YetAnotherConfigLib", "isXander", 21_900_000, ANY, "library", "A builder-based configuration library for Minecraft."],
  ["5ZwdcRci", "immediatelyfast", "ImmediatelyFast", "RaphiMC", 23_400_000, ANY, "optimization", "Speed up immediate mode rendering in Minecraft."],
  ["fQEb0iXm", "krypton", "Krypton", "astei", 12_800_000, FQ, "optimization", "A mod to optimize the Minecraft networking stack."],
  ["uLbm7CG6", "dynamic-fps", "Dynamic FPS", "juliand665", 16_200_000, ANY, "optimization", "Reduce resource usage while Minecraft is in the background, idle or on battery."],
  ["H8CaAYZC", "starlight", "Starlight", "Spottedleaf", 15_100_000, FQ, "optimization", "Rewrites the light engine to fix lighting performance and lighting errors.", { mc: ["1.20.1", "1.19.4", "1.19.2", "1.18.2"] }],
  ["EsAfCjCV", "appleskin", "AppleSkin", "squeek502", 33_500_000, ANY, "utility", "Food/hunger-related HUD improvements."],
  ["LQ3K71Q1", "dynamic-lights", "LambDynamicLights", "LambdAurora", 14_900_000, FQ, "decoration", "Dynamic lights for Minecraft."],
  ["fRiHVvU7", "canvas", "Canvas Renderer", "grondag", 2_300_000, FQ, "optimization", "Shader-based renderer for Minecraft with Fabric.", { incompatible: ["AANobbMI"], mc: ["1.20.1", "1.19.4"] }],
  ["Orvt0mRa", "indium", "Indium", "comp500", 13_600_000, FQ, "library", "Sodium addon providing support for the Fabric Rendering API.", { requires: ["AANobbMI"], mc: ["1.20.4", "1.20.1"] }],
  ["9eGKb6K1", "simple-voice-chat", "Simple Voice Chat", "henkelmax", 29_400_000, ANY, "social", "A working voice chat in Minecraft!"],
  ["XaDC71GB", "xaeros-world-map", "Xaero's World Map", "xaero96", 26_700_000, ANY, "map", "Adds a self-writing world map to the game."],
  ["GchcoXML", "oculus", "Oculus", "Asek3", 9_800_000, ["forge"], "decoration", "A fork of Iris for Forge.", { requires: ["sk9rgfiA"], mc: ["1.20.1", "1.19.2", "1.18.2"] }],
  ["sk9rgfiA", "embeddium", "Embeddium", "embeddedt", 12_200_000, FORGES, "optimization", "Rendering optimization mod for Forge and NeoForge based on Sodium.", { mc: ["1.21.1", "1.20.1", "1.19.2", "1.18.2"] }],
  // Borderless fullscreen mods (real Modrinth ids, see lib/borderless.ts).
  ["UIexcMP1", "borderless-fullscreen", "Borderless Fullscreen", "Ideal", 2_900_000, ["fabric"], "utility", "Makes fullscreen borderless.", { mc: ["1.21.10", "1.21.8", "1.21.5", "1.21.4", "1.21.1", "1.20.1"] }],
  ["kYq5qkSL", "borderless-mining", "Borderless Mining", "comp500", 9_500_000, ["fabric", "quilt", "forge"], "utility", "Changes the fullscreen option to a borderless window.", { mc: ["1.20.1", "1.19.4", "1.19.2", "1.18.2", "1.16.5"] }],
  ["KxYAzYLi", "borderless-mining-updated", "Borderless Mining Updated", "community", 800_000, ["fabric", "neoForge"], "utility", "Borderless fullscreen for recent versions.", { mc: ["1.21.10"] }],
  ["CnalMsa0", "borderless-window", "Borderless Window", "community", 210_000, FORGES, "utility", "Borderless windowed fullscreen for Forge and NeoForge.", { mc: ["1.21.1", "1.20.1", "1.19.2", "1.18.2", "1.16.5"] }],
];

const MR_PACKS: Row[] = [
  ["50dA9Sha", "fresh-animations", "Fresh Animations", "FreshLX", 9_200_000, [], "vanilla-like", "A resource pack that adds animations to mobs."],
  ["N6n5dqoA", "faithful-32x", "Faithful 32x", "Faithful Resource Pack", 7_600_000, [], "32x", "A Minecraft resource pack that doubles the resolution while keeping the default look."],
  ["HhyzKxEa", "stay-true", "Stay True", "haughtydatum", 3_400_000, [], "vanilla-like", "A vanilla-style texture pack that adds variety while staying true."],
  ["ID4NhWQ6", "better-leaves", "Better Leaves", "Motschen", 4_800_000, [], "tweaks", "Makes leaves look bushier."],
  ["sW96TpBE", "xali-enchanted-books", "Xali's Enchanted Books", "Xali", 4_100_000, [], "utility", "Changes the look of enchanted books to make them unique per enchantment."],
  ["XnNJvSqN", "low-on-fire", "Low On Fire", "Juknum", 5_900_000, [], "tweaks", "Lowers fire on screen."],
];

const MR_SHADERS: Row[] = [
  ["HVnmMxH1", "complementary-reimagined", "Complementary Shaders - Reimagined", "EminGT", 16_900_000, [], "realistic", "A shaderpack that aims to be complementary to vanilla."],
  ["Q1vvjJYV", "bsl-shaders", "BSL Shaders", "CaptTatsu", 13_200_000, [], "semi-realistic", "Bright, soft and lively shader pack."],
  ["R6NEzAwj", "sildurs-vibrant", "Sildur's Vibrant Shaders", "Sildur", 6_300_000, [], "realistic", "Colorful shaders with volumetric light."],
  ["BVzZfTc1", "makeup-ultra-fast", "MakeUp - Ultra Fast", "XavierFST", 5_200_000, [], "potato", "A shaderpack focused on performance."],
  ["7CJ7EJNU", "solas", "Solas Shader", "Septonious", 2_700_000, [], "fantasy", "A soft and colorful shaderpack."],
];

const CF_MODS: Row[] = [
  ["238222", "jei", "Just Enough Items (JEI)", "mezz", 362_000_000, ANY, "api,utility", "View Items and Recipes."],
  ["328085", "create", "Create", "simibubi", 128_000_000, FORGES, "technology", "Aesthetic Technology that empowers the Player.", { mc: ["1.21.1", "1.20.1", "1.19.2", "1.18.2", "1.16.5"] }],
  ["263420", "xaeros-minimap", "Xaero's Minimap", "xaero96", 141_000_000, ANY, "map", "Displays the nearby world terrain, players, mobs, entities in the corner of your screen.", { manual: true }],
  ["220318", "biomes-o-plenty", "Biomes O' Plenty", "Forstride", 117_000_000, ANY, "world-gen", "Adds over 50 new biomes.", { requires: ["419699"] }],
  ["419699", "terrablender", "TerraBlender", "Adubbz", 96_000_000, ANY, "library", "A library mod for adding biomes in a simple and compatible manner."],
  ["223794", "applied-energistics-2", "Applied Energistics 2", "thetechnici4n", 89_000_000, FORGES, "technology,storage", "A Mod about Matter, Energy and using them to conjure items from thin air!"],
  ["245755", "waystones", "Waystones", "BlayTheNinth", 102_000_000, ANY, "transportation", "Teleport back to activated waystones.", { requires: ["531761"] }],
  ["531761", "balm", "Balm", "BlayTheNinth", 99_000_000, ANY, "library", "Abstraction layer for multiloader mods."],
  ["60089", "mouse-tweaks", "Mouse Tweaks", "YaLTeR", 207_000_000, ANY, "utility", "Enhances inventory management by adding various functions to the mouse buttons."],
  ["306612", "fabric-api", "Fabric API", "modmuss50", 160_000_000, FQ, "library", "Core API module providing key hooks and intercompatibility features."],
  ["351725", "storage-drawers", "Storage Drawers", "Texelsaur", 77_000_000, FORGES, "storage", "Drawers that store items.", { manual: true }],
];

const CF_PACKS: Row[] = [
  ["351052", "fresh-animations-cf", "Fresh Animations", "FreshLX", 6_100_000, [], "16x", "A resource pack that adds animations to mobs."],
  ["236822", "faithful-32x-cf", "Faithful 32x", "Faithful Resource Pack", 9_500_000, [], "32x", "Doubles the resolution of the default textures."],
];

const CF_SHADERS: Row[] = [
  ["627557", "complementary-reimagined-cf", "Complementary Shaders - Reimagined", "EminGT", 12_300_000, [], "realistic", "A shaderpack that aims to be complementary to vanilla."],
  ["385587", "bsl-shaders-cf", "BSL Shaders", "CaptTatsu", 20_100_000, [], "semi-realistic", "Bright, soft and lively shader pack."],
];

const MR_MODPACKS: Row[] = [
  ["1KVo5zza", "fabulously-optimized", "Fabulously Optimized", "robotkoer", 14_800_000, ["fabric"], "optimization,lightweight", "Improved performance, nicer graphics and modern features, while still feeling like vanilla.", { mc: ["1.21.10", "1.21.8", "1.21.5", "1.21.4", "1.21.1", "1.20.1"] }],
  ["ZsLzSFnK", "cobblemon-fabric", "Cobblemon Official Modpack", "Cobblemon", 3_100_000, ["fabric"], "adventure,multiplayer", "The official Cobblemon modpack: catch, train and battle in Minecraft.", { mc: ["1.21.1", "1.20.1"] }],
  ["8Bf7rCjA", "better-mc-neoforge", "Better MC [NEOFORGE]", "SHXRKIE", 2_400_000, ["neoForge"], "adventure,quests", "Improves vanilla with hundreds of mods: new biomes, structures, mobs and quests.", { mc: ["1.21.1"] }],
  ["lR5jBg3k", "simply-optimized", "Simply Optimized", "Sky", 1_900_000, ["fabric", "quilt"], "optimization", "A simple, stable and highly compatible optimization modpack.", { mc: ["1.21.4", "1.21.1", "1.20.1"] }],
];

const CF_MODPACKS: Row[] = [
  ["715572", "all-the-mods-9", "All the Mods 9", "ATMTeam", 9_600_000, ["forge"], "kitchen-sink,tech,magic", "All the Mods started out as a private pack for just a few friends of ours that turned into something others wanted to play.", { mc: ["1.20.1"] }],
  ["452013", "rlcraft", "RLCraft", "Shivaxi", 21_300_000, ["forge"], "hardcore,adventure", "A modpack specially designed to bring an incredibly hardcore and semi-realism challenge.", { mc: ["1.12.2"], manual: true }],
  ["520914", "create-above-and-beyond", "Create: Above and Beyond", "simibubi", 6_700_000, ["forge"], "tech,quests", "An expert-ish modpack themed around Create and automation.", { mc: ["1.16.5"] }],
];

export const CATALOG: MockProject[] = [
  ...MR_MODS.map((r, i) => project("modrinth", "mod", r, i)),
  ...MR_MODPACKS.map((r, i) => project("modrinth", "modpack", r, i + 90)),
  ...CF_MODPACKS.map((r, i) => project("curseForge", "modpack", r, i + 95)),
  ...MR_PACKS.map((r, i) => project("modrinth", "resourcePack", r, i + 30)),
  ...MR_SHADERS.map((r, i) => project("modrinth", "shader", r, i + 40)),
  ...CF_MODS.map((r, i) => project("curseForge", "mod", r, i + 50)),
  ...CF_PACKS.map((r, i) => project("curseForge", "resourcePack", r, i + 70)),
  ...CF_SHADERS.map((r, i) => project("curseForge", "shader", r, i + 80)),
];

export function findProject(source: ContentSourceKind, projectId: string): MockProject | undefined {
  return CATALOG.find((p) => p.source === source && p.projectId === projectId);
}

/** Mods and modpacks are tied to a loader and a Minecraft version. */
export const loaderBound = (kind: ContentKind): boolean => kind === "mod" || kind === "modpack";

/** Loaders only matter for mods and modpacks; packs and shaders fit any instance. */
function loaderFits(p: MockProject, loader: LoaderKind | null): boolean {
  if (!loaderBound(p.kind) || loader === null) return true;
  if (loader === "vanilla") return false;
  return p.loaders.includes(loader);
}

export function searchCatalog(q: SearchQuery): SearchPage {
  const text = q.text.trim().toLowerCase();
  const score = (p: MockProject) =>
    text === "" ? 0 : p.title.toLowerCase().startsWith(text) ? 2 : p.title.toLowerCase().includes(text) ? 1 : 0;
  const hits = CATALOG.filter(
    (p) =>
      p.source === q.source &&
      p.kind === q.kind &&
      (text === "" ||
        p.title.toLowerCase().includes(text) ||
        p.slug.includes(text) ||
        p.description.toLowerCase().includes(text)) &&
      (q.mc === null || !loaderBound(p.kind) || p.mc.includes(q.mc)) &&
      loaderFits(p, q.loader) &&
      (q.categories.length === 0 || q.categories.some((c) => p.categories.includes(c))),
  );
  const sorted = [...hits].sort((a, b) => {
    switch (q.sort) {
      case "downloads":
        return b.downloads - a.downloads;
      case "updated":
        return b.updated.localeCompare(a.updated);
      case "newest":
        return b.created.localeCompare(a.created);
      default:
        return score(b) - score(a) || b.downloads - a.downloads;
    }
  });
  const items = sorted.slice(q.offset, q.offset + q.limit).map((p): ProjectSummary => ({
    source: p.source,
    projectId: p.projectId,
    slug: p.slug,
    title: p.title,
    description: p.description,
    author: p.author,
    iconUrl: p.iconUrl,
    downloads: p.downloads,
    kind: p.kind,
    categories: [...p.categories],
  }));
  return { items, total: sorted.length, offset: q.offset };
}

function hashNum(s: string): number {
  let h = 2166136261;
  for (const ch of s) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return h >>> 0;
}

function fileExt(p: MockProject): string {
  if (p.kind === "modpack") return p.source === "modrinth" ? "mrpack" : "zip";
  return p.kind === "mod" ? "jar" : "zip";
}

/** Every version of a project, newest first. */
export function projectVersions(p: MockProject): ContentVersion[] {
  const out: ContentVersion[] = [];
  const mcs = p.mc.slice(0, 8);
  const base = hashNum(p.projectId) % 7;
  mcs.forEach((mc, mi) => {
    for (let rev = 0; rev < 2; rev += 1) {
      const versionNumber = `${1 + (base % 3)}.${9 - mi}.${base + (1 - rev)}${p.kind === "mod" ? `+${mc}` : ""}`;
      const versionId = `${p.projectId}-${mc}-${rev}`;
      const published = new Date(Date.parse(p.updated) - (mi * 40 + rev * 15) * 86_400_000).toISOString();
      out.push({
        source: p.source,
        projectId: p.projectId,
        versionId,
        name: `${p.title} ${versionNumber}`,
        versionNumber,
        mcVersions: [mc],
        loaders: loaderBound(p.kind) ? [...p.loaders] : [],
        published,
        fileName: `${p.slug}-${versionNumber.replace("+", "-")}.${fileExt(p)}`,
        size: 80_000 + (hashNum(versionId) % 12_000_000),
        sha1: hashNum(versionId).toString(16).padStart(8, "0").repeat(5),
        dependencies: [
          ...p.requires.map((id) => ({ projectId: id, versionId: null, kind: "required" as const })),
          ...p.incompatible.map((id) => ({ projectId: id, versionId: null, kind: "incompatible" as const })),
        ],
        manualDownload: p.manual,
      });
    }
  });
  return out;
}

export function filterVersions(list: ContentVersion[], filter: ContentFilter): ContentVersion[] {
  return list.filter(
    (v) =>
      (filter.mc === null || v.mcVersions.includes(filter.mc)) &&
      (filter.loader === null || v.loaders.length === 0 || v.loaders.includes(filter.loader)),
  );
}

export function manualUrl(p: MockProject, v: ContentVersion): string {
  const section =
    p.kind === "mod" ? "mc-mods" : p.kind === "shader" ? "shaders" : p.kind === "modpack" ? "modpacks" : "texture-packs";
  return `https://www.curseforge.com/minecraft/${section}/${p.slug}/files/${hashNum(v.versionId) % 9_000_000}`;
}

// ---- installed content ------------------------------------------------------

function installedFrom(
  source: ContentSourceKind,
  projectId: string,
  mc: string,
  loader: LoaderKind,
  opts: { enabled?: boolean; update?: boolean } = {},
): InstalledContent | null {
  const p = findProject(source, projectId);
  if (!p) return null;
  const v = filterVersions(projectVersions(p), { mc, loader: p.kind === "mod" ? loader : null });
  const pick = opts.update ? v[1] : v[0];
  if (!pick) return null;
  return {
    fileName: pick.fileName,
    kind: p.kind,
    enabled: opts.enabled ?? true,
    source,
    projectId,
    versionId: pick.versionId,
    title: p.title,
    versionNumber: pick.versionNumber,
    updateAvailable: opts.update ?? false,
  };
}

const nonNull = <T,>(x: T | null): x is T => x !== null;

export function seedInstalled(): Record<string, InstalledContent[]> {
  const fab = (id: string, o?: { enabled?: boolean; update?: boolean }) =>
    installedFrom("modrinth", id, "1.21.1", "fabric", o);
  const big: InstalledContent[] = Array.from({ length: 140 }, (_, i) => ({
    fileName: `addon-${String(i + 1).padStart(3, "0")}-1.20.1.jar`,
    kind: "mod" as const,
    enabled: i % 17 !== 0,
    source: (i % 3 === 0 ? "curseForge" : "modrinth") as ContentSourceKind,
    projectId: `big-${i}`,
    versionId: `big-${i}-v1`,
    title: `Addon ${i + 1}`,
    versionNumber: `1.${i % 9}.${i % 4}`,
    updateAvailable: i % 11 === 0,
  }));
  return {
    "inst-survival": [
      installedFrom("modrinth", "50dA9Sha", "1.21.10", "vanilla"),
      installedFrom("modrinth", "HhyzKxEa", "1.21.10", "vanilla", { enabled: false }),
    ].filter(nonNull),
    "inst-fabric": [
      fab("P7dR8mSH"),
      fab("AANobbMI"),
      fab("gvQqBUqZ"),
      fab("YL57xq9U"),
      fab("mOgUt4GM", { update: true }),
      fab("w7ThoJFB", { enabled: false }),
      fab("1eAoo2KR"),
      <InstalledContent>{
        fileName: "custom-tweaks-1.0.jar",
        kind: "mod",
        enabled: true,
        source: "local",
        projectId: null,
        versionId: null,
        title: null,
        versionNumber: null,
        updateAvailable: false,
      },
      fab("50dA9Sha"),
      fab("HVnmMxH1", { update: true }),
    ].filter(nonNull),
    "inst-big": big,
  };
}

// ---- loader versions --------------------------------------------------------

const isRelease = (mc: string) => /^\d+\.\d+(\.\d+)?$/.test(mc);

function mcMinor(mc: string): [number, number] {
  const [, minor = 0, patch = 0] = mc.split(".").map((n) => Number(n));
  return [minor, patch];
}

export function mockLoaderVersions(loader: LoaderKind, mc: string): LoaderVersion[] {
  const mk = (versions: string[]): LoaderVersion[] =>
    versions.map((version, i) => ({ loader, version, mc, stable: i !== 0 || loader === "forge", recommended: i === 1 }));
  const [minor, patch] = isRelease(mc) ? mcMinor(mc) : [99, 0];
  switch (loader) {
    case "vanilla":
      return [];
    case "fabric":
      return minor >= 14 ? mk(["0.17.3", "0.17.2", "0.16.14", "0.16.10", "0.15.11"]) : [];
    case "quilt":
      return minor >= 14 ? mk(["0.29.2-beta.3", "0.29.1", "0.28.4", "0.27.1"]) : [];
    case "forge": {
      if (!isRelease(mc)) return [];
      const major = 26 + minor + (minor >= 20 ? patch : 0);
      return mk([`${major}.4.10`, `${major}.4.0`, `${major}.3.12`, `${major}.2.0`]).map((v) => ({
        ...v,
        version: `${mc}-${v.version}`,
      }));
    }
    case "neoForge": {
      if (!isRelease(mc) || minor < 20 || (minor === 20 && patch < 2)) return [];
      const pre = `${minor}.${patch}`;
      return mk([`${pre}.215-beta`, `${pre}.209`, `${pre}.200`, `${pre}.180`]);
    }
  }
}
