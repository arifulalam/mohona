import type { Api, Category, Config, ConfigChange, EngineStatus, ExtensionSettings, Details, Plan, PlanWindow, Queue, Row, Rule } from "./types";

/** True inside the Tauri window; false in a plain browser (development, tests). */
export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function tauriApi(): Api {
  const call = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      return await invoke<T>(command, args);
    } catch (e) {
      throw new Error(typeof e === "string" ? e : String((e as Error)?.message ?? e));
    }
  };
  return {
    status: () => call<EngineStatus>("engine_status"),
    start: () => call<EngineStatus>("start_engine"),
    list: () => call<Row[]>("list_downloads"),
    add: (url, folder, queue) => call<number>("add_download", { url, folder, queue }),
    details: (id) => call<Details>("get_details", { id }),
    rules: () => call<Rule[]>("get_rules"),
    setRules: (rules) => call<Rule[]>("set_rules", { rules }),
    queues: () => call<Queue[]>("get_queues"),
    addQueue: (name) => call<Queue[]>("add_queue", { name }),
    renameQueue: (id, name) => call<Queue[]>("rename_queue", { id, name }),
    removeQueue: (id) => call<Queue[]>("remove_queue", { id }),
    setQueue: (id, change) => call<Queue[]>("set_queue", { id, change }),
    pause: (id) => call<void>("pause_download", { id }),
    resume: (id) => call<void>("resume_download", { id }),
    cancel: (id) => call<void>("cancel_download", { id }),
    remove: (id, deleteFile) => call<void>("remove_download", { id, deleteFile }),
    open: (id) => call<void>("open_download", { id }),
    show: (id) => call<void>("show_download", { id }),
    plan: () => call<Plan>("get_plan"),
    setPlan: (windows) => call<Plan>("set_plan", { windows }),
    setAfterAction: (action, countdownSeconds) => call<Plan>("set_after_action", { action, countdownSeconds }),
    cancelAction: () => call<Plan>("cancel_action"),
    pickFolder: () => call<string | null>("pick_folder"),
    clipboard: () => call<string | null>("read_clipboard"),
    categories: () => call<Category[]>("get_categories"),
    addCategory: (name, extensions, folder) => call<Category[]>("add_category", { name, extensions, folder }),
    removeCategory: (id) => call<Category[]>("remove_category", { id }),
    config: () => call<Config>("get_config"),
    setConfig: (change) => call<Config>("set_config", { change }),
    extensionSettings: () => call<ExtensionSettings>("get_extension_settings"),
    setExtensionSettings: (settings) => call<ExtensionSettings>("set_extension_settings", { settings }),
  };
}

/** A pretend engine, so the window can be tried in a browser. */
export function demoApi(tick = 1000): Api {
  let running = false;
  let next = 4;
  let rows: Row[] = [
    demoRow(1, "ubuntu-24.04-desktop-amd64.iso", "downloading", 3_100_000_000, 6_200_000_000, 8_400_000),
    demoRow(2, "lecture-07.mp4", "paused", 120_000_000, 480_000_000, 0),
    demoRow(3, "dataset.zip", "completed", 52_000_000, 52_000_000, 0),
  ];
  setInterval(() => {
    if (!running) return;
    rows = rows.map((r) => {
      if (r.status !== "downloading" || r.size === null) return r;
      const downloaded = Math.min(r.size, r.downloaded + r.speed * (tick / 1000));
      return downloaded >= r.size
        ? { ...r, downloaded, percent: 100, speed: 0, eta: null, status: "completed" }
        : { ...r, downloaded, percent: (downloaded / r.size) * 100, eta: Math.ceil((r.size - downloaded) / r.speed) };
    });
  }, tick);
  let config: Config = { downloadFolder: "C:\\Users\\you\\Downloads", defaultFolder: "C:\\Users\\you\\Downloads", speedLimit: null, maxActive: 4, notifications: true, keepInTray: true, sortByType: false };
  let ext: ExtensionSettings = { intercept: true, minSizeBytes: 0, excludedHosts: [], videoButton: true, buttonHiddenHosts: [], autoRefresh: true };
  let cats: Category[] = [
    { id: 1, name: "Compressed", extensions: ["zip", "rar", "7z"], folder: null },
    { id: 2, name: "Video", extensions: ["mp4", "mkv", "webm"], folder: null },
  ];
  let nextCat = 10;
  let queueList: Queue[] = [{ id: 1, name: "Main", maxActive: 4, speedLimit: null, windows: [], openNow: true }];
  let nextQueue = 2;
  let ruleList: Rule[] = [];
  let nextRule = 1;
  const checkName = (name: string) => {
    const n = name.trim();
    if (!n || n.length > 40) throw new Error("a queue's name must be 1 to 40 characters");
    if (queueList.some((q) => q.name.toLowerCase() === n.toLowerCase())) throw new Error("there is already a queue with that name");
    return n;
  };
  let plan: Plan = { windows: [], openNow: true, action: null, countdown: null, message: null };
  const status = (): EngineStatus => ({ running, version: running ? "demo" : null, message: null });
  const need = () => {
    if (!running) throw new Error("Mohona is not running");
  };
  const set = (id: number, patch: Partial<Row>) => {
    rows = rows.map((r) => (r.id === id ? { ...r, ...patch, speed: patch.status === "downloading" ? 5_000_000 : 0 } : r));
  };
  return {
    status: async () => status(),
    start: async () => ((running = true), status()),
    list: async () => (need(), rows.map((r) => ({ ...r }))),
    add: async (url) => {
      need();
      const id = next++;
      const name = decodeURIComponent(url.split("?")[0]!.split("/").filter(Boolean).pop() ?? "download");
      rows = [...rows, demoRow(id, name, "downloading", 0, 300_000_000, 5_000_000)];
      return id;
    },
    pause: async (id) => (need(), set(id, { status: "paused", eta: null })),
    resume: async (id) => (need(), set(id, { status: "downloading" })),
    cancel: async (id) => (need(), set(id, { status: "canceled", downloaded: 0, percent: 0, eta: null })),
    remove: async (id) => (need(), void (rows = rows.filter((r) => r.id !== id))),
    open: async (id) => {
      need();
      if (/\.(exe|msi|bat|cmd|ps1|lnk)$/i.test(rows.find((r) => r.id === id)?.name ?? "")) throw new Error("for safety, programs and scripts are not opened from here; use \"Show in folder\"");
    },
    show: async () => need(),
    details: async (id) => {
      need();
      const row = rows.find((r) => r.id === id);
      if (!row) throw new Error("there is no such download");
      const size = row.size ?? 0;
      const live = row.status === "downloading" && size > 0;
      const part = Math.ceil(size / 4);
      return {
        id,
        status: row.status,
        downloaded: row.downloaded,
        size: row.size,
        speed: row.speed,
        connections: live
          ? [0, 1, 2, 3].map((i) => {
              const start = i * part;
              const end = Math.min(size, start + part);
              const received = Math.min(end - start, Math.max(0, row.downloaded - start));
              return { number: i + 1, start, end, received, speed: received >= end - start ? 0 : Math.round(row.speed / 3), state: received >= end - start ? "done" : "active" };
            })
          : [],
      };
    },
    rules: async () => (need(), ruleList),
    setRules: async (rules) => {
      need();
      rules.forEach((r, n) => {
        if (!r.name.trim()) throw new Error(`rule ${n + 1}: the name must be 1 to 60 characters`);
        if (!r.host && !r.extensions.length && !r.urlContains) throw new Error(`rule ${n + 1}: give at least one condition, or the rule would match everything`);
        if (!r.block && !r.folder && r.queue === null && r.sort === null) throw new Error(`rule ${n + 1}: give it something to do`);
        if (r.folder && /(^|[\\/])\.\.([\\/]|$)/.test(r.folder)) throw new Error(`rule ${n + 1}: a folder name inside the download folder cannot use ".."`);
      });
      return (ruleList = rules.map((r) => ({ ...r, id: r.id > 0 ? r.id : nextRule++ })));
    },
    queues: async () => (need(), queueList),
    addQueue: async (name) => (need(), (queueList = [...queueList, { id: nextQueue++, name: checkName(name), maxActive: 3, speedLimit: null, windows: [], openNow: true }])),
    renameQueue: async (id, name) => (need(), (queueList = queueList.map((q) => (q.id === id ? { ...q, name: checkName(name) } : q)))),
    removeQueue: async (id) => {
      need();
      if (id === queueList[0]?.id) throw new Error("the main queue cannot be removed");
      return (queueList = queueList.filter((q) => q.id !== id));
    },
    setQueue: async (id, c) => {
      need();
      if (c.maxActive !== undefined && (c.maxActive < 1 || c.maxActive > 32)) throw new Error("simultaneous downloads must be between 1 and 32");
      return (queueList = queueList.map((q) => (q.id === id ? { ...q, maxActive: c.maxActive ?? q.maxActive, speedLimit: c.speedLimit === undefined ? q.speedLimit : c.speedLimit || null, windows: c.windows ?? q.windows } : q)));
    },
    plan: async () => (need(), plan),
    setPlan: async (windows: PlanWindow[]) => {
      need();
      if (windows.some((w) => w.start === w.end)) throw new Error("a window must not start and end at the same time");
      return (plan = { ...plan, windows });
    },
    setAfterAction: async (action) => (need(), (plan = { ...plan, action, countdown: null, message: null })),
    cancelAction: async () => (need(), (plan = { ...plan, action: null, countdown: null })),
    clipboard: async () => navigator.clipboard?.readText().then((t) => t || null, () => null) ?? null,
    pickFolder: async () => window.prompt("Folder (the real app opens a folder dialog here):", "C:\\Users\\you\\Downloads\\Videos"),
    categories: async () => (need(), cats),
    addCategory: async (name, extensions, folder) => {
      need();
      const taken = cats.find((c) => c.extensions.some((e) => extensions.includes(e)));
      if (taken) throw new Error(`"${taken.extensions.find((e) => extensions.includes(e))}" already belongs to ${taken.name}`);
      cats = [...cats, { id: nextCat++, name, extensions, folder }];
      return cats;
    },
    removeCategory: async (id) => (need(), (cats = cats.filter((c) => c.id !== id))),
    config: async () => (need(), config),
    setConfig: async (c: ConfigChange) => {
      need();
      if (c.maxActive !== undefined && (c.maxActive < 1 || c.maxActive > 32)) throw new Error("simultaneous downloads must be between 1 and 32");
      config = {
        ...config,
        downloadFolder: c.downloadFolder === undefined ? config.downloadFolder : c.downloadFolder || config.defaultFolder,
        speedLimit: c.speedLimit === undefined ? config.speedLimit : c.speedLimit || null,
        maxActive: c.maxActive ?? config.maxActive,
        notifications: c.notifications ?? config.notifications,
        keepInTray: c.keepInTray ?? config.keepInTray,
        sortByType: c.sortByType ?? config.sortByType,
      };
      return config;
    },
    extensionSettings: async () => (need(), ext),
    setExtensionSettings: async (s) => (need(), (ext = s)),
  };
}

function demoRow(id: number, name: string, status: string, downloaded: number, size: number, speed: number): Row {
  return {
    id,
    name,
    url: `https://example.com/files/${name}`,
    status,
    downloaded,
    size,
    percent: (downloaded / size) * 100,
    speed,
    eta: speed > 0 ? Math.ceil((size - downloaded) / speed) : null,
    error: null,
    path: status === "completed" ? `C:\\Users\\you\\Downloads\\${name}` : null,
    expired: false,
  };
}

export function chooseApi(): Api {
  return insideTauri() ? tauriApi() : demoApi();
}
