import type { Api, Category, Config, ConfigChange, EngineStatus, ExtensionSettings, Details, Plan, PlanWindow, Queue, Row, Rule, SiteProfile, TorrentList, ToolInfo, Seeding, GrabState, GrabFile, GrabAdded, ScanSetting, SharedTorrent, Startup, VideoFormat, UpdateInfo, TorrentSpeed } from "./types";

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
    add: (url, folder, queue, files) => call<number>("add_download", { url, folder, queue, files }),
    torrentFiles: (url) => call<TorrentList>("torrent_files", { url }),
    grabStart: (options) => call<void>("grab_start", { options }),
    grabStatus: () => call<GrabState>("grab_status"),
    grabStop: () => call<void>("grab_stop"),
    grabDownload: (files, folder, queue) => call<GrabAdded>("grab_download", { files, folder, queue }),
    details: (id) => call<Details>("get_details", { id }),
    tools: () => call<ToolInfo[]>("get_tools"),
    setTool: (name, path) => call<ToolInfo[]>("set_tool", { name, path }),
    openToolPage: (url) => call<void>("open_tool_page", { url }),
    videoFormat: () => call<VideoFormat>("get_video_format"),
    setVideoFormat: (c) => call<VideoFormat>("set_video_format", { ...c }),
    sharing: () => call<SharedTorrent[]>("get_sharing"),
    stopSharing: (id) => call<SharedTorrent[]>("stop_sharing", { id }),
    torrentSpeed: () => call<TorrentSpeed>("get_torrent_speed"),
    setTorrentSpeed: (s) => call<TorrentSpeed>("set_torrent_speed", { downKb: s.downKb, upKb: s.upKb }),
    seeding: () => call<Seeding>("get_seeding"),
    setSeeding: (s) => call<Seeding>("set_seeding", { ratio: s.ratio, minutes: s.minutes }),
    update: () => call<UpdateInfo>("get_update"),
    setUpdateRepo: (repo) => call<UpdateInfo>("set_update_repo", { repo }),
    checkUpdate: () => call<UpdateInfo>("check_update"),
    installUpdate: () => call<string>("install_update"),
    openReleasePage: (url) => call<void>("open_release_page", { url }),
    openExtensionFolder: (browser) => call<void>("open_extension_folder", { browser }),
    setDropTarget: (on) => call<void>("set_drop_target", { on }),
    showMain: () => call<void>("show_main"),
    startup: () => call<Startup>("get_startup"),
    setStartup: (on) => call<Startup>("set_startup", { on }),
    scan: () => call<ScanSetting>("get_scan"),
    setScan: (mode, command) => call<ScanSetting>("set_scan", { mode, command }),
    videoBrowser: () => call<string | null>("get_video_browser"),
    setVideoBrowser: (browser) => call<string | null>("set_video_browser", { browser }),
    profiles: () => call<SiteProfile[]>("get_profiles"),
    setProfiles: (profiles) => call<SiteProfile[]>("set_profiles", { profiles }),
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
  let next = 9;
  let rows: Row[] = [
    demoRow(1, "ubuntu-24.04-desktop-amd64.iso", "downloading", 3_100_000_000, 6_200_000_000, 8_400_000),
    demoRow(2, "lecture-07.mp4", "paused", 120_000_000, 480_000_000, 0),
    demoRow(3, "dataset.zip", "completed", 52_000_000, 52_000_000, 0),
    demoRow(4, "Folk Tune - Audio Jukebox [x7Qe2.mp4]", "downloading", 88_000_000, 210_000_000, 3_600_000),
    demoRow(5, "design-handbook.pdf", "completed", 18_400_000, 18_400_000, 0),
    demoRow(6, "Big.Buck.Bunny.1080p.torrent", "downloading", 410_000_000, 1_100_000_000, 12_800_000),
    demoRow(7, "setup-visual-studio-code.exe", "completed", 96_000_000, 96_000_000, 0),
    demoRow(8, "podcast-episode-42.mp3", "queued", 0, 64_000_000, 0),
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
    { id: 3, name: "Music", extensions: ["mp3", "flac", "m4a"], folder: null },
    { id: 4, name: "Documents", extensions: ["pdf", "docx", "txt"], folder: null },
    { id: 5, name: "Programs", extensions: ["exe", "msi", "iso"], folder: null },
    { id: 6, name: "Torrents", extensions: ["torrent"], folder: null },
  ];
  let nextCat = 10;
  let queueList: Queue[] = [{ id: 1, name: "Main", maxActive: 4, speedLimit: null, windows: [], openNow: true }];
  let nextQueue = 2;
  let ruleList: Rule[] = [];
  let nextRule = 1;
  let demoGrab: GrabState = { state: "idle", pages: 0, found: 0, current: "", files: [], skippedByRobots: 0, failedPages: 0, note: null, error: null };
const demoGrabFiles = (url: string): GrabFile[] => [
  { url: new URL("photo.jpg", url).href, name: "photo.jpg", group: "images", page: url },
  { url: new URL("guide.pdf", url).href, name: "guide.pdf", group: "documents", page: url },
  { url: new URL("files/all.zip", url).href, name: "all.zip", group: "archives", page: url },
];
let demoBrowser: string | null = null;
let demoStartup: Startup = { available: false, enabled: false };
let demoScan: ScanSetting = { mode: "off", command: "", defender: null };
  let demoSpeed: TorrentSpeed = { downKb: null, upKb: null };
  let demoSeeding: Seeding = { ratio: null, minutes: null };
  let toolList: ToolInfo[] = [
    { name: "ffmpeg", purpose: "join separate video and audio, add subtitles to a video, convert files, and merge the best video and sound of a YouTube download", chosen: null, found: null, page: "https://ffmpeg.org/download.html", windowsPage: "https://www.gyan.dev/ffmpeg/builds/", winget: "winget install Gyan.FFmpeg" },
    { name: "yt-dlp", purpose: "download videos from YouTube, Vimeo and similar sites (they hide the real file behind a web page that needs a program to read it)", chosen: null, found: null, page: "https://github.com/yt-dlp/yt-dlp#installation", windowsPage: "https://github.com/yt-dlp/yt-dlp/releases/latest", winget: "winget install yt-dlp.yt-dlp" },
  ];
  let profileList: SiteProfile[] = [];
  let nextProfile = 1;
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
    grabStart: async (options) => {
      need();
      demoGrab = { ...demoGrab, state: "done", pages: 2, found: 3, files: demoGrabFiles(options.url) };
    },
    grabStatus: async () => (need(), demoGrab),
    grabStop: async () => undefined,
    grabDownload: async (files) => (need(), { added: files.length, skipped: [] }),
    torrentFiles: async () => {
      need();
      await new Promise((r) => setTimeout(r, 300));
      return {
        name: "Example distribution",
        files: [
          { index: 0, path: "Example distribution/disc.iso", size: 2_400_000_000 },
          { index: 1, path: "Example distribution/README.txt", size: 4_200 },
          { index: 2, path: "Example distribution/extras/docs.pdf", size: 18_000_000 },
        ],
      };
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
    tools: async () => (need(), toolList.map((t) => ({ ...t }))),
    setTool: async (name, path) => {
      need();
      if (path && !/ffmpeg|yt-dlp/i.test(path)) throw new Error(`${path} is not ${name} (give the file, or the folder that holds it)`);
      toolList = toolList.map((t) => (t.name === name ? { ...t, chosen: path, found: path } : t));
      return toolList.map((t) => ({ ...t }));
    },
    openToolPage: async (url) => void window.open(url, "_blank", "noopener"),
    videoFormat: async () => ({ quality: "best", container: "mp4", audio: "mp3" }),
    setVideoFormat: async (c) => c,
    sharing: async () => [],
    stopSharing: async () => [],
    torrentSpeed: async () => (need(), { ...demoSpeed }),
    setTorrentSpeed: async (s) => ((demoSpeed = { downKb: s.downKb || null, upKb: s.upKb || null }), { ...demoSpeed }),
    seeding: async () => (need(), { ...demoSeeding }),
    setSeeding: async (s) => ((demoSeeding = { ratio: s.ratio || null, minutes: s.minutes || null }), { ...demoSeeding }),
    update: async () => (need(), { repo: "", current: "0.1.0", latest: null, newer: false, url: null, notes: null, installable: false }),
    setUpdateRepo: async (repo) => ({ repo, current: "0.1.0", latest: null, newer: false, url: null, notes: null, installable: false }),
    checkUpdate: async () => { throw new Error("The demo cannot look for updates."); },
    installUpdate: async () => "0.2.0",
    openReleasePage: async () => undefined,
    openExtensionFolder: async () => undefined,
    setDropTarget: async () => undefined,
    showMain: async () => undefined,
    startup: async () => (need(), demoStartup),
    setStartup: async (on) => ((demoStartup = { ...demoStartup, enabled: on }), demoStartup),
    scan: async () => (need(), demoScan),
    setScan: async (mode, command) => ((demoScan = { mode: mode as ScanSetting["mode"], command, defender: null }), demoScan),
    videoBrowser: async () => (need(), demoBrowser),
    setVideoBrowser: async (browser) => ((demoBrowser = browser), demoBrowser),
    profiles: async () => (need(), profileList),
    setProfiles: async (profiles) => {
      need();
      profiles.forEach((p, n) => {
        if (!p.name.trim()) throw new Error(`profile ${n + 1}: the name must be 1 to 60 characters`);
        if (!/^[a-z0-9.*_-]+$/i.test(p.host.trim()) || p.host.trim() === "*") throw new Error(`profile ${n + 1}: the site is a name such as example.com (or *.example.com), not an address`);
        if (p.connections !== null && (p.connections < 1 || p.connections > 32)) throw new Error(`profile ${n + 1}: connections must be from 1 to 32`);
        if (p.connections === null && !p.speedLimit && !p.userAgent && !p.referrer && !p.headers.length) throw new Error(`profile ${n + 1}: give it something to set`);
      });
      return (profileList = profiles.map((p) => ({ ...p, host: p.host.trim().toLowerCase(), id: p.id > 0 ? p.id : nextProfile++ })));
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
