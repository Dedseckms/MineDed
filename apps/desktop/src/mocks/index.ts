// Mock backend for UI work without Rust. Owned by ui.
// Stateful: instances, accounts, settings and tasks live in memory; long
// operations stream `TaskEvent`s with progress and emit the same events as
// the real backend through the mock bus.
// Every command not implemented here resolves to a `notImplemented` error,
// so the mock always satisfies the full generated `commands` type.
import type {
  Account,
  AppError,
  Instance,
  InstanceId,
  RunState,
  TaskEvent,
  TaskKind,
  TaskSummary,
  commands,
} from "../bindings";
import { validateNick } from "../lib/validation";
import { type MockBus, mockBus } from "./bus";

export { mockBus };
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
    appVersion: async () => "0.0.0-mock",

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
      if (spec.loader.kind !== "vanilla") return fail(appError("notImplemented", "mock: loaders"));
      const version = VERSIONS.find((v) => v.id === spec.mc);
      if (!version) return notFound(`version ${spec.mc}`);
      const id = runTask(
        "instanceCreate",
        "tasks.instanceCreate",
        ch,
        [
          { stage: "stages.versionMeta", total: null },
          { stage: "stages.client", total: 24_000_000 },
          { stage: "stages.libraries", total: 48_000_000 },
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
      const root = settings.gameDir ?? "C:\\Users\\Player\\AppData\\Roaming\\MineUP";
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
  };
  return impl;
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
