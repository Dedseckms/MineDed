// Mock backend for UI work without Rust. Owned by ui.
// Stateful: instances, accounts, settings and tasks live in memory; long
// operations stream `TaskEvent`s with progress and emit the same events as
// the real backend through the mock bus.
// Every command not implemented here resolves to a `notImplemented` error,
// so the mock always satisfies the full generated `commands` type.
import type {
  Account,
  AppError,
  ContentSourceKind,
  InstalledContent,
  LocalDepIssue,
  Instance,
  InstanceId,
  ResolvePlan,
  RunState,
  Skin,
  TaskEvent,
  TaskKind,
  TaskSummary,
  commands,
} from "../bindings";
import {
  IMPORT_EXTS,
  SKIN_EXTS,
  validateFilePath,
  validateNick,
  validateSkinName,
  validateSkinSource,
} from "../lib/validation";
import type { ContentAddLocal } from "../api/localContent";
import { LOCAL_EXTS, isLocalKind, pickLoaderVersion } from "../lib/content";
import { fileName } from "../lib/path";
import { type MockBus, mockBus } from "./bus";
import {
  filterVersions,
  findProject,
  loaderBound,
  mockLoaderVersions,
  projectVersions,
  searchCatalog,
  seedInstalled,
} from "./content";
import { createNetMocks } from "./net";
import { paletteFromSeed, seedCapes, seedSkins, skinDataUrl } from "./skins";
import { MOCK_CURRENT_VERSION, createUpdateMocks } from "./updates";

export { mockBus };
export { demoOverrides, parseDemoParams, runDemo } from "./demo";
export { MOCK_UPDATE, mockUpdateControl } from "./updates";
import {
  GAME_LOG_SAMPLE,
  SYSTEM_INFO,
  VERSIONS,
  YGGDRASIL_SERVERS,
  defaultSettings,
  seedAccounts,
  seedInstances,
} from "./data";

type Commands = typeof commands;
type Channel = Parameters<Commands["instanceLaunch"]>[2];

export interface MockOptions {
  bus?: MockBus;
  /** Interval between progress events, ms. */
  tickMs?: number;
  /** Artificial latency of every command, ms. */
  latencyMs?: number;
}

const appError = (code: AppError["code"], message: string): AppError => ({
  code,
  message,
  details: null,
});

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const fail = (error: AppError) => ({ status: "error" as const, error });

/** CurseForge is turned off: the backend answers like this for any CurseForge request. */
const curseForgeOff = () =>
  fail({ ...appError("authRequired", "mock: CurseForge is disabled"), details: { reason: "curseforgeDisabled" } });

const notImplemented = (name: string) => async () => fail(appError("notImplemented", `mock: ${name}`));

interface Stage {
  stage: string;
  total: number | null;
}

interface MockTask {
  summary: TaskSummary;
  cancelled: boolean;
}

function slug(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9а-яё]+/gi, "-")
      .replace(/^-|-$/g, "") || "instance"
  );
}

export function createMockBackend(opts: MockOptions = {}): Partial<Commands> {
  const bus = opts.bus ?? mockBus;
  const tickMs = opts.tickMs ?? 150;
  const latencyMs = opts.latencyMs ?? 0;

  let instances = seedInstances();
  let accounts = seedAccounts();
  let settings = defaultSettings();
  const installed = seedInstalled();
  let skins = seedSkins();
  const capes = seedCapes();
  const activeCape = new Map<string, string | null>();
  const runStates = new Map<InstanceId, RunState>();
  const logTimers = new Map<InstanceId, ReturnType<typeof setInterval>>();
  const tasks = new Map<string, MockTask>();
  let seq = 0;

  const wait = () =>
    latencyMs > 0 ? new Promise<void>((r) => setTimeout(r, latencyMs)) : Promise.resolve();

  const tasksList = (): TaskSummary[] => [...tasks.values()].map((t) => ({ ...t.summary }));
  const notifyTasks = () => bus.emit("tasksChanged", tasksList());
  const notifyAccounts = () => bus.emit("accountChanged", accounts.map((a) => ({ ...a })));

  const setRunState = (id: InstanceId, state: RunState, exitCode: number | null = null) => {
    runStates.set(id, state);
    bus.emit("instanceState", { id, state, exitCode, crashReport: null });
  };

  const stopLog = (id: InstanceId) => {
    const timer = logTimers.get(id);
    if (timer) clearInterval(timer);
    logTimers.delete(id);
  };

  const startLog = (id: InstanceId) => {
    stopLog(id);
    let i = 0;
    const timer = setInterval(() => {
      const t = new Date().toTimeString().slice(0, 8);
      const line = GAME_LOG_SAMPLE[i % GAME_LOG_SAMPLE.length] ?? "";
      i += 1;
      bus.emit("instanceLog", { id, lines: [`[${t}] ${line}`] });
    }, tickMs * 4);
    logTimers.set(id, timer);
  };

  /** Simulates a long task: started → progress per stage → finished. */
  function runTask(
    kind: TaskKind,
    titleKey: string,
    ch: Channel,
    stages: Stage[],
    onSuccess: () => AppError | null,
  ): string {
    seq += 1;
    const id = `task-${seq}`;
    const task: MockTask = {
      summary: { id, kind, titleKey, status: "running", done: 0, total: null, stage: "" },
      cancelled: false,
    };
    tasks.set(id, task);
    const send = (ev: TaskEvent) => {
      if (ev.type === "progress") {
        task.summary = { ...task.summary, done: ev.done, total: ev.total, stage: ev.stage };
      }
      ch.onmessage(ev);
    };
    const finish = (error: AppError | null) => {
      task.summary = {
        ...task.summary,
        status: error === null ? "succeeded" : error.code === "cancelled" ? "cancelled" : "failed",
      };
      send({ type: "finished", id, error });
      notifyTasks();
    };

    send({ type: "started", id, kind, titleKey });
    notifyTasks();

    let stageIdx = 0;
    let done = 0;
    const step = () => {
      if (task.cancelled) return finish(appError("cancelled", "mock: cancelled"));
      const stage = stages[stageIdx];
      if (!stage) return finish(onSuccess());
      const total = stage.total;
      const chunk = total === null ? 1 : Math.ceil(total / 6);
      done = total === null ? done + 1 : Math.min(total, done + chunk);
      send({
        type: "progress",
        id,
        done,
        total,
        stage: stage.stage,
        speedBps: total === null ? null : Math.round((chunk * 1000) / tickMs),
      });
      if (total === null ? done >= 3 : done >= total) {
        send({ type: "log", id, line: `${stage.stage}: ok` });
        stageIdx += 1;
        done = 0;
      }
      setTimeout(step, tickMs);
    };
    setTimeout(step, tickMs);
    return id;
  }

  const findInstance = (id: InstanceId) => instances.find((i) => i.id === id);
  const notFound = (what: string) => fail(appError("notFound", `mock: ${what} not found`));

  const impl: Partial<Commands> = {
    appVersion: async () => MOCK_CURRENT_VERSION,

    versionsList: async (filter) => {
      await wait();
      const search = filter.search?.trim().toLowerCase() ?? "";
      return ok(
        VERSIONS.filter(
          (v) =>
            (filter.kinds.length === 0 || filter.kinds.includes(v.kind)) &&
            (search === "" || v.id.toLowerCase().includes(search)),
        ),
      );
    },

    systemInfo: async () => {
      await wait();
      return ok({ ...SYSTEM_INFO });
    },

    ramRecommend: async (id) => {
      await wait();
      if (!findInstance(id)) return notFound(`instance ${id}`);
      return ok({ minMb: 1024, maxMb: 4096, reasonKey: "memory.reason.default" });
    },

    // ---- instances ----
    instanceList: async () => {
      await wait();
      return ok(instances.map((i) => ({ ...i })));
    },

    instanceGet: async (id) => {
      await wait();
      const inst = findInstance(id);
      return inst ? ok({ ...inst }) : notFound(`instance ${id}`);
    },

    instanceCreate: async (spec, ch) => {
      await wait();
      if (spec.name.trim() === "") return fail(appError("invalidInput", "mock: empty name"));
      const version = VERSIONS.find((v) => v.id === spec.mc);
      if (!version) return notFound(`version ${spec.mc}`);
      if (spec.loader.kind !== "vanilla") {
        const known = mockLoaderVersions(spec.loader.kind, spec.mc);
        if (!spec.loader.version || !known.some((v) => v.version === spec.loader.version))
          return fail(appError("invalidInput", "mock: unknown loader version"));
      }
      const id = runTask(
        "instanceCreate",
        "tasks.instanceCreate",
        ch,
        [
          { stage: "stages.versionMeta", total: null },
          { stage: "stages.client", total: 24_000_000 },
          { stage: "stages.libraries", total: 48_000_000 },
          ...(spec.loader.kind !== "vanilla" ? [{ stage: "stages.libraries", total: null }] : []),
          { stage: "stages.assets", total: 96_000_000 },
        ],
        () => {
          instances = [
            ...instances,
            {
              id: `inst-${slug(spec.name)}-${seq}`,
              name: spec.name.trim(),
              mc: spec.mc,
              loader: spec.loader,
              java: { path: null, major: version.javaMajor },
              memory: { ...settings.defaultMemory },
              jvmArgs: [...settings.defaultJvmArgs],
              preset: spec.preset,
              icon: spec.icon,
              createdAt: new Date().toISOString(),
              lastPlayed: null,
              playtimeSec: 0,
            },
          ];
          return null;
        },
      );
      return ok(id);
    },

    instanceClone: async (srcId, name, ch) => {
      await wait();
      const src = findInstance(srcId);
      if (!src) return notFound(`instance ${srcId}`);
      const id = runTask(
        "instanceCreate",
        "tasks.instanceClone",
        ch,
        [{ stage: "stages.copy", total: 120_000_000 }],
        () => {
          instances = [
            ...instances,
            {
              ...structuredClone(src),
              id: `inst-${slug(name)}-${seq}`,
              name: name.trim(),
              createdAt: new Date().toISOString(),
              lastPlayed: null,
              playtimeSec: 0,
            },
          ];
          return null;
        },
      );
      return ok(id);
    },

    instanceUpdate: async (id, patch) => {
      await wait();
      const inst = findInstance(id);
      if (!inst) return notFound(`instance ${id}`);
      if (patch.memory && patch.memory.maxMb < patch.memory.minMb)
        return fail(appError("invalidInput", "mock: maxMb < minMb"));
      const next: Instance = {
        ...inst,
        name: patch.name ?? inst.name,
        java: patch.java ?? inst.java,
        memory: patch.memory ?? inst.memory,
        jvmArgs: patch.jvmArgs ?? inst.jvmArgs,
        icon: patch.icon ?? inst.icon,
      };
      instances = instances.map((i) => (i.id === id ? next : i));
      return ok({ ...next });
    },

    instanceDelete: async (id) => {
      await wait();
      if (!findInstance(id)) return notFound(`instance ${id}`);
      const state = runStates.get(id);
      if (state === "running" || state === "preparing")
        return fail(appError("conflict", "mock: instance is running"));
      instances = instances.filter((i) => i.id !== id);
      return ok(null);
    },

    instanceGameDir: async (id) => {
      await wait();
      const inst = findInstance(id);
      if (!inst) return notFound(`instance ${id}`);
      const root = settings.gameDir ?? "C:\\Users\\Player\\AppData\\Roaming\\MineDed";
      return ok(`${root}\\instances\\${slug(inst.name)}`);
    },

    instanceState: async (id) => {
      await wait();
      return ok(runStates.get(id) ?? "stopped");
    },

    instanceLaunch: async (id, launchOpts, ch) => {
      await wait();
      const inst = findInstance(id);
      if (!inst) return notFound(`instance ${id}`);
      const state = runStates.get(id);
      if (state === "running" || state === "preparing")
        return fail(appError("conflict", "mock: already running"));
      const account = launchOpts.account
        ? accounts.find((a) => a.id === launchOpts.account)
        : accounts.find((a) => a.isDefault);
      if (!account) return fail(appError("authRequired", "mock: no account"));
      setRunState(id, "preparing");
      const taskId = runTask(
        "launch",
        "tasks.launch",
        ch,
        [
          { stage: "stages.java", total: null },
          { stage: "stages.download", total: 12_000_000 },
          { stage: "stages.assets", total: 30_000_000 },
        ],
        () => {
          setRunState(id, "running");
          instances = instances.map((i) =>
            i.id === id ? { ...i, lastPlayed: new Date().toISOString() } : i,
          );
          startLog(id);
          return null;
        },
      );
      // A cancelled/failed launch leaves the instance stopped.
      const watch = setInterval(() => {
        const t = tasks.get(taskId);
        if (!t || t.summary.status === "running") return;
        clearInterval(watch);
        if (t.summary.status !== "succeeded") setRunState(id, "stopped");
      }, tickMs);
      return ok(taskId);
    },

    instanceKill: async (id) => {
      await wait();
      const state = runStates.get(id) ?? "stopped";
      if (state === "stopped") return ok(null);
      stopLog(id);
      bus.emit("instanceLog", { id, lines: ["[Render thread/INFO]: Stopping!"] });
      setRunState(id, "stopped", 1);
      return ok(null);
    },

    // ---- accounts ----
    accountList: async () => {
      await wait();
      return ok(accounts.map((a) => ({ ...a })));
    },

    accountOfflineAdd: async (name) => {
      await wait();
      if (validateNick(name) !== null) return fail(appError("invalidInput", "mock: bad nickname"));
      if (accounts.some((a) => a.kind === "offline" && a.name.toLowerCase() === name.toLowerCase()))
        return fail(appError("conflict", "mock: duplicate nickname"));
      seq += 1;
      const account: Account = {
        id: `acc-${seq}`,
        kind: "offline",
        name,
        uuid: crypto.randomUUID(),
        server: null,
        skinUrl: null,
        isDefault: accounts.length === 0,
        valid: true,
      };
      accounts = [...accounts, account];
      notifyAccounts();
      return ok({ ...account });
    },

    accountYggdrasilServers: async () => {
      await wait();
      return ok(YGGDRASIL_SERVERS.map((s) => ({ ...s })));
    },

    accountYggdrasilLogin: async () => {
      await wait();
      return fail(appError("notImplemented", "mock: yggdrasil login"));
    },

    accountRemove: async (id) => {
      await wait();
      const acc = accounts.find((a) => a.id === id);
      if (!acc) return notFound(`account ${id}`);
      accounts = accounts.filter((a) => a.id !== id);
      const first = accounts[0];
      if (acc.isDefault && first) accounts = accounts.map((a) => ({ ...a, isDefault: a.id === first.id }));
      notifyAccounts();
      return ok(null);
    },

    accountSetDefault: async (id) => {
      await wait();
      if (!accounts.some((a) => a.id === id)) return notFound(`account ${id}`);
      accounts = accounts.map((a) => ({ ...a, isDefault: a.id === id }));
      notifyAccounts();
      return ok(null);
    },

    accountYggdrasilRegister: async () => {
      await wait();
      return fail(appError("notImplemented", "mock: yggdrasil register"));
    },

    // ---- loaders & content ----
    loaderVersions: async (loader, mc) => {
      await wait();
      return ok(mockLoaderVersions(loader, mc));
    },

    contentSearch: async (query) => {
      await wait();
      if (query.source === "curseForge") return curseForgeOff();
      if (query.limit < 1 || query.limit > 100 || query.offset < 0)
        return fail(appError("invalidInput", "mock: bad paging"));
      return ok(searchCatalog(query));
    },

    contentVersions: async (source, projectId, filter) => {
      await wait();
      if (source === "curseForge") return curseForgeOff();
      const p = findProject(source, projectId);
      if (!p) return notFound(`project ${projectId}`);
      return ok(filterVersions(projectVersions(p), loaderBound(p.kind) ? filter : { ...filter, loader: null }));
    },

    contentList: async (instanceId) => {
      await wait();
      if (!findInstance(instanceId)) return notFound(`instance ${instanceId}`);
      return ok((installed[instanceId] ?? []).map((c) => ({ ...c })));
    },

    contentSetEnabled: async (instanceId, fileName, enabled) => {
      await wait();
      const list = installed[instanceId];
      if (!list?.some((c) => c.fileName === fileName)) return notFound(`file ${fileName}`);
      installed[instanceId] = list.map((c) => (c.fileName === fileName ? { ...c, enabled } : c));
      return ok(null);
    },

    contentRemove: async (instanceId, fileName) => {
      await wait();
      const list = installed[instanceId];
      if (!list?.some((c) => c.fileName === fileName)) return notFound(`file ${fileName}`);
      installed[instanceId] = list.filter((c) => c.fileName !== fileName);
      return ok(null);
    },

    // Issues are derived from file names: "addon" → needs Fabric API, "old" →
    // wants a newer Sodium, "forge" on a non-Forge instance → wrong loader,
    // "conflict" → clashes with an installed Sodium.
    contentCheckLocal: async (instanceId, fileNames) => {
      await wait();
      const inst = findInstance(instanceId);
      if (!inst) return notFound(`instance ${instanceId}`);
      const have = (installed[instanceId] ?? []).filter((c) => c.enabled);
      const has = (projectId: string) => have.some((c) => c.projectId === projectId);
      const issues: LocalDepIssue[] = [];
      for (const fileName of fileNames) {
        const n = fileName.toLowerCase();
        if (!n.endsWith(".jar")) continue;
        if (n.includes("forge") && inst.loader.kind !== "forge" && inst.loader.kind !== "neoForge") {
          issues.push({ fileName, kind: "wrongLoader", modId: "forge", versionReq: null, suggestion: null, suggestionTitle: null });
          continue;
        }
        if (n.includes("addon") && !has("P7dR8mSH")) {
          issues.push({
            fileName,
            kind: "missing",
            modId: "fabric-api",
            versionReq: "*",
            suggestion: { source: "modrinth", projectId: "P7dR8mSH", versionId: null },
            suggestionTitle: "Fabric API",
          });
        }
        if (n.includes("old")) {
          issues.push({
            fileName,
            kind: has("AANobbMI") ? "wrongVersion" : "missing",
            modId: "sodium",
            versionReq: ">=0.6.0",
            suggestion: { source: "modrinth", projectId: "AANobbMI", versionId: null },
            suggestionTitle: "Sodium",
          });
        }
        if (n.includes("conflict")) {
          issues.push({ fileName, kind: "conflict", modId: "sodium", versionReq: null, suggestion: null, suggestionTitle: null });
        }
      }
      return ok(issues);
    },

    contentResolve: async (instanceId, picks) => {
      await wait();
      const inst = findInstance(instanceId);
      if (!inst) return notFound(`instance ${instanceId}`);
      const have = installed[instanceId] ?? [];
      const plan: ResolvePlan = { install: [], alreadyInstalled: [], conflicts: [], manual: [] };
      const seen = new Set<string>();
      const queue: Array<{ source: ContentSourceKind; projectId: string; versionId: string | null }> = [...picks];
      while (queue.length > 0) {
        const pick = queue.shift();
        if (!pick || seen.has(pick.projectId)) continue;
        seen.add(pick.projectId);
        const p = findProject(pick.source, pick.projectId);
        if (!p) return notFound(`project ${pick.projectId}`);
        const all = projectVersions(p);
        const compatible = filterVersions(all, { mc: inst.mc, loader: p.kind === "mod" ? inst.loader.kind : null });
        const v = pick.versionId ? all.find((x) => x.versionId === pick.versionId) : compatible[0];
        if (!v) {
          const e = appError("notFound", `mock: no compatible version of ${p.projectId}`);
          return fail({ ...e, details: { projectId: p.projectId, title: p.title } });
        }
        const current = have.find((c) => c.projectId === p.projectId);
        if (current && (current.versionId === v.versionId || !picks.some((x) => x.projectId === p.projectId))) {
          plan.alreadyInstalled.push(p.projectId);
          continue;
        }
        for (const bad of p.incompatible) {
          const other = have.find((c) => c.projectId === bad) ?? plan.install.find((x) => x.projectId === bad);
          if (other) plan.conflicts.push({ projectId: p.projectId, withProjectId: bad, reason: "incompatible" });
        }
        plan.install.push(v);
        for (const dep of p.requires) queue.push({ source: p.source, projectId: dep, versionId: null });
      }
      return ok(plan);
    },

    contentInstall: async (instanceId, plan, ch) => {
      await wait();
      const inst = findInstance(instanceId);
      if (!inst) return notFound(`instance ${instanceId}`);
      if (plan.conflicts.length > 0) return fail(appError("conflict", "mock: plan has conflicts"));
      const total = plan.install.reduce((s, v) => s + v.size, 0);
      const id = runTask("contentInstall", "tasks.contentInstall", ch, [{ stage: "stages.download", total: Math.max(total, 1) }], () => {
        const list = installed[instanceId] ?? [];
        const added: InstalledContent[] = plan.install.map((v) => {
          const p = findProject(v.source, v.projectId);
          return {
            fileName: v.fileName,
            kind: p?.kind ?? "mod",
            enabled: true,
            source: v.source,
            projectId: v.projectId,
            versionId: v.versionId,
            title: p?.title ?? v.name,
            versionNumber: v.versionNumber,
            updateAvailable: false,
          };
        });
        const ids = new Set(added.map((a) => a.projectId));
        installed[instanceId] = [...list.filter((c) => !c.projectId || !ids.has(c.projectId)), ...added];
        return null;
      });
      return ok(id);
    },

    contentUpdateAll: async (instanceId, ch) => {
      await wait();
      if (!findInstance(instanceId)) return notFound(`instance ${instanceId}`);
      const count = (installed[instanceId] ?? []).filter((c) => c.updateAvailable).length;
      const id = runTask("contentUpdate", "tasks.contentUpdate", ch, [{ stage: "stages.download", total: Math.max(1, count) * 2_000_000 }], () => {
        installed[instanceId] = (installed[instanceId] ?? []).map((c) =>
          c.updateAvailable ? { ...c, updateAvailable: false, versionNumber: `${c.versionNumber ?? "1.0"}.1` } : c,
        );
        return null;
      });
      return ok(id);
    },

    presetApply: async (instanceId, preset, ch) => {
      await wait();
      if (!findInstance(instanceId)) return notFound(`instance ${instanceId}`);
      const id = runTask("presetApply", "tasks.presetApply", ch, [{ stage: "stages.download", total: null }], () => {
        instances = instances.map((i) => (i.id === instanceId ? { ...i, preset } : i));
        return null;
      });
      return ok(id);
    },

    instanceImport: async (path, ch) => {
      await wait();
      if (validateFilePath(path, IMPORT_EXTS) !== null) return fail(appError("invalidInput", "mock: bad pack path"));
      if (/missing/i.test(path)) return fail({ ...appError("notFound", "mock: file not found"), details: { path } });
      const mrpack = path.toLowerCase().endsWith(".mrpack");
      const name = (path.split(/[\\/]/).pop() ?? "Pack").replace(/\.(mrpack|zip)$/i, "");
      const id = runTask(
        "instanceImport",
        "tasks.instanceImport",
        ch,
        [
          { stage: "stages.copy", total: null },
          { stage: "stages.download", total: 64_000_000 },
          { stage: "stages.libraries", total: null },
        ],
        () => {
          const instId = `inst-${slug(name)}-${seq}`;
          instances = [
            ...instances,
            {
              id: instId,
              name: name.slice(0, 64),
              mc: mrpack ? "1.21.1" : "1.20.1",
              loader: mrpack ? { kind: "fabric", version: "0.16.14" } : { kind: "forge", version: "1.20.1-47.4.0" },
              java: { path: null, major: mrpack ? 21 : 17 },
              memory: { ...settings.defaultMemory },
              jvmArgs: [],
              preset: null,
              icon: null,
              createdAt: new Date().toISOString(),
              lastPlayed: null,
              playtimeSec: 0,
            },
          ];
          installed[instId] = [];
          return null;
        },
      );
      return ok(id);
    },

    modpackInstall: async (source, projectId, versionId, ch) => {
      await wait();
      if (source === "curseForge") return curseForgeOff();
      const p = findProject(source, projectId);
      if (!p || p.kind !== "modpack") return notFound(`modpack ${projectId}`);
      const versions = projectVersions(p);
      const v = versionId === null ? versions[0] : versions.find((x) => x.versionId === versionId);
      if (!v) return notFound(`modpack version ${versionId ?? "latest"}`);
      const id = runTask(
        "instanceImport",
        "tasks.modpackInstall",
        ch,
        [
          { stage: "stages.download", total: v.size },
          { stage: "stages.download", total: 96_000_000 },
          { stage: "stages.libraries", total: null },
        ],
        () => {
          const mc = v.mcVersions[0] ?? "1.21.1";
          const kind = v.loaders[0] ?? "vanilla";
          const instId = `inst-${slug(p.title)}-${seq}`;
          instances = [
            ...instances,
            {
              id: instId,
              name: p.title.slice(0, 64),
              mc,
              loader: { kind, version: kind === "vanilla" ? null : pickLoaderVersion(mockLoaderVersions(kind, mc)) },
              java: { path: null, major: VERSIONS.find((x) => x.id === mc)?.javaMajor ?? 17 },
              memory: { ...settings.defaultMemory },
              jvmArgs: [],
              preset: null,
              icon: null,
              createdAt: new Date().toISOString(),
              lastPlayed: null,
              playtimeSec: 0,
            },
          ];
          installed[instId] = [];
          return null;
        },
      );
      return ok(id);
    },

    instanceExport: async (instanceId, format, path, ch) => {
      await wait();
      if (!findInstance(instanceId)) return notFound(`instance ${instanceId}`);
      const ext = format === "mrpack" ? [".mrpack"] : [".zip"];
      if (validateFilePath(path, ext) !== null) return fail(appError("invalidInput", "mock: bad export path"));
      const id = runTask("instanceExport", "tasks.instanceExport", ch, [{ stage: "stages.copy", total: 40_000_000 }], () => null);
      return ok(id);
    },

    // ---- skins ----
    skinLibrary: async () => {
      await wait();
      return ok(skins.map((s) => ({ ...s })));
    },

    // Mock "cache files" from `skinFetchRemote` are data URLs (previewable without Tauri).
    skinAdd: async (pngPath, name, model) => {
      await wait();
      const cached = pngPath.startsWith("data:image/png");
      if ((!cached && validateFilePath(pngPath, SKIN_EXTS) !== null) || validateSkinName(name) !== null)
        return fail(appError("invalidInput", "mock: bad skin"));
      if (/missing/i.test(pngPath)) return fail({ ...appError("notFound", "mock: file not found"), details: { path: pngPath } });
      seq += 1;
      const skin: Skin = {
        id: `skin-${seq}`,
        name: name.trim(),
        model,
        hash: `${seq}`.padStart(64, "0"),
        url: cached ? pngPath : skinDataUrl(paletteFromSeed(`${pngPath}:${name}`), model),
        localPath: cached ? null : pngPath.trim(),
        localOnly: true,
      };
      skins = [...skins, skin];
      return ok({ ...skin });
    },

    // Nicknames/links containing "missing" are not found; the model is slim for names ending in "a".
    skinFetchRemote: async (source) => {
      await wait();
      const s = source.trim();
      if (validateSkinSource(s) !== null) return fail(appError("invalidInput", "mock: bad skin source"));
      if (/missing/i.test(s)) return fail({ ...appError("notFound", "mock: skin not found"), details: { source: s } });
      const isUrl = /^https?:/i.test(s);
      const base = isUrl ? (s.split(/[/?#]/).filter(Boolean).pop() ?? "skin").replace(/\.png$/i, "") : s;
      const model = /a$/i.test(base) ? "slim" : "classic";
      return ok({
        localPath: skinDataUrl(paletteFromSeed(`remote:${s}`), model),
        model,
        suggestedName: base.slice(0, 32) || "skin",
      });
    },

    skinRemove: async (id) => {
      await wait();
      if (!skins.some((s) => s.id === id)) return notFound(`skin ${id}`);
      skins = skins.filter((s) => s.id !== id);
      return ok(null);
    },

    skinApply: async (accountId, skinId) => {
      await wait();
      const skin = skins.find((s) => s.id === skinId);
      if (!skin) return notFound(`skin ${skinId}`);
      if (!accounts.some((a) => a.id === accountId)) return notFound(`account ${accountId}`);
      accounts = accounts.map((a) => (a.id === accountId ? { ...a, skinUrl: skin.url } : a));
      notifyAccounts();
      return ok(null);
    },

    capeList: async (accountId) => {
      await wait();
      if (!accounts.some((a) => a.id === accountId)) return notFound(`account ${accountId}`);
      const active = activeCape.get(accountId) ?? null;
      return ok(capes.map((c) => ({ ...c, active: c.id === active })));
    },

    capeSet: async (accountId, capeId) => {
      await wait();
      if (!accounts.some((a) => a.id === accountId)) return notFound(`account ${accountId}`);
      if (capeId !== null && !capes.some((c) => c.id === capeId)) return notFound(`cape ${capeId}`);
      activeCape.set(accountId, capeId);
      return ok(null);
    },

    // ---- settings ----
    settingsGet: async () => {
      await wait();
      return ok(structuredClone(settings));
    },

    settingsSet: async (next) => {
      await wait();
      if (next.defaultMemory.maxMb < next.defaultMemory.minMb)
        return fail(appError("invalidInput", "mock: maxMb < minMb"));
      if (next.proxy && (next.proxy.host === "" || next.proxy.port < 1))
        return fail(appError("invalidInput", "mock: bad proxy"));
      settings = structuredClone(next);
      return ok(null);
    },

    // ---- tasks ----
    tasksList: async () => {
      await wait();
      return ok(tasksList());
    },

    taskCancel: async (id) => {
      await wait();
      const t = tasks.get(id);
      if (!t) return notFound(`task ${id}`);
      t.cancelled = true;
      return ok(null);
    },

    // ---- vpn, rooms, chat ----
    ...createNetMocks({
      bus,
      tickMs,
      wait,
      findInstance,
      addInstance: (inst) => {
        instances = [...instances, inst];
        installed[inst.id] = [];
      },
      findAccount: (id) => accounts.find((a) => a.id === id),
      runTask,
      selfName: () => (accounts.find((a) => a.isDefault) ?? accounts[0])?.name ?? "Player",
    }),

    // ---- self-update ----
    ...createUpdateMocks({ bus, wait, runTask }),
  };

  // `content_add_local` is not in the generated bindings yet: kept outside the
  // `Partial<Commands>` literal so it typechecks before and after regeneration.
  const contentAddLocal: ContentAddLocal = async (instanceId, kind, paths) => {
    await wait();
    if (!findInstance(instanceId)) return notFound(`instance ${instanceId}`);
    if (!isLocalKind(kind)) return fail(appError("invalidInput", `mock: kind ${kind} cannot be added from disk`));
    const exts: readonly string[] = LOCAL_EXTS[kind];
    const names = paths.map(fileName);
    if (names.length === 0 || names.some((n) => !exts.some((e) => n.toLowerCase().endsWith(`.${e}`)))) {
      return fail(appError("invalidInput", "mock: unsupported file type"));
    }
    if (paths.some((p) => /missing/i.test(p))) return notFound("file");
    const added: InstalledContent[] = names.map((n) => ({
      fileName: n,
      kind,
      enabled: true,
      source: "local",
      projectId: null,
      versionId: null,
      title: null,
      versionNumber: null,
      updateAvailable: false,
    }));
    const keep = (installed[instanceId] ?? []).filter((c) => !names.includes(c.fileName));
    installed[instanceId] = [...keep, ...added];
    return ok(added.map((c) => ({ ...c })));
  };
  return Object.assign(impl, { contentAddLocal });
}

export function createMockCommands(extra: Partial<Commands> = {}, opts: MockOptions = {}): Commands {
  const impl: Partial<Commands> = { ...createMockBackend(opts), ...extra };
  return new Proxy(impl, {
    get(target, prop: string | symbol) {
      // Not a thenable: the object is resolved from a Promise (lazy loading in client.ts).
      if (typeof prop === "symbol" || prop === "then") return undefined;
      return target[prop as keyof Commands] ?? notImplemented(prop);
    },
  }) as Commands;
}
