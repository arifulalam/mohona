/** One line of the download list (what the Rust side sends). */
export interface Row {
  id: number;
  name: string;
  url: string;
  status: string;
  downloaded: number;
  size: number | null;
  percent: number | null;
  speed: number;
  eta: number | null;
  error: string | null;
  path: string | null;
  expired: boolean;
}

export interface EngineStatus {
  running: boolean;
  version: string | null;
  message: string | null;
}

/** The engine's own settings. */
export interface Config {
  downloadFolder: string;
  defaultFolder: string;
  /** Bytes per second; null for no limit. */
  speedLimit: number | null;
  maxActive: number;
  notifications: boolean;
  keepInTray: boolean;
  sortByType: boolean;
}

export interface ConfigChange {
  downloadFolder?: string;
  /** Bytes per second; 0 removes the limit. */
  speedLimit?: number;
  maxActive?: number;
  notifications?: boolean;
  keepInTray?: boolean;
  sortByType?: boolean;
}

/** A group of file types sorted into its own folder. */
export interface Category {
  id: number;
  name: string;
  extensions: string[];
  /** As set; null means a folder named like the category. */
  folder: string | null;
}

/** One time window of the schedule. */
export interface PlanWindow {
  /** mon … sun; empty means every day. */
  days: string[];
  start: string;
  end: string;
  /** Bytes per second while the window is open; null keeps the usual limit. */
  speedLimit: number | null;
}

export interface Plan {
  windows: PlanWindow[];
  openNow: boolean;
  /** sleep, hibernate or shutdown, when armed. */
  action: string | null;
  /** Seconds left while the countdown runs. */
  countdown: number | null;
  message: string | null;
}

/** One download queue. */
export interface Queue {
  id: number;
  name: string;
  maxActive: number;
  /** Bytes per second for the queue together; null for no limit. */
  speedLimit: number | null;
  windows: PlanWindow[];
  openNow: boolean;
}

export interface QueueChange {
  maxActive?: number;
  /** Bytes per second; 0 removes the limit. */
  speedLimit?: number;
  windows?: PlanWindow[];
}

/** One file inside a torrent. */
export interface TorrentFile {
  index: number;
  path: string;
  size: number;
}

/** What the site grabber is to look for. */
export interface GrabOptions {
  url: string;
  /** 0: this page only; 1: also the pages it links to; ... (at most 5) */
  depth: number;
  sameSite: boolean;
  /** Groups (`images`, `video`, `audio`, `documents`, `archives`, `programs`) or endings; none: every file. */
  types: string[];
  maxPages: number | null;
  robots: boolean;
}

/** A file the site grabber found. */
export interface GrabFile {
  url: string;
  name: string;
  group: string;
  /** The page that links to it. */
  page: string;
}

/** How the site grabber is doing, and what it found. */
export interface GrabState {
  state: "idle" | "running" | "done" | "stopped" | "failed";
  pages: number;
  found: number;
  current: string;
  files: GrabFile[];
  skippedByRobots: number;
  failedPages: number;
  note: string | null;
  error: string | null;
}

export interface GrabAdded {
  added: number;
  skipped: string[];
}

/** What a torrent holds. */
export interface TorrentList {
  name: string;
  files: TorrentFile[];
}

/** How long finished torrents are shared. */
export interface Seeding {
  /** Stop at this upload ratio, in hundredths (150 is 1.5). */
  ratio: number | null;
  /** Stop after this many minutes. */
  minutes: number | null;
}

/** An outside program Mohona can use (ffmpeg, yt-dlp). */
export interface ToolInfo {
  name: string;
  /** What Mohona needs it for. */
  purpose: string;
  /** The path the person chose, if any. */
  chosen: string | null;
  /** The file that will be used, if one is found. */
  found: string | null;
  page: string;
  windowsPage: string;
  winget: string;
}

/** One rule: what to look for in a new download, and what to do with it. */
export interface Rule {
  /** 0 for a rule that is not saved yet. */
  id: number;
  name: string;
  enabled: boolean;
  host: string | null;
  extensions: string[];
  urlContains: string | null;
  folder: string | null;
  queue: number | null;
  sort: boolean | null;
  block: boolean;
}

/** How downloads from one site are made. */
export interface SiteProfile {
  /** 0 for a profile that is not saved yet. */
  id: number;
  name: string;
  enabled: boolean;
  host: string;
  connections: number | null;
  /** Bytes per second for each download from the site; null for no limit. */
  speedLimit: number | null;
  userAgent: string | null;
  referrer: string | null;
  headers: [string, string][];
}

/** One connection of a running download. */
export interface Connection {
  number: number;
  start: number;
  end: number;
  received: number;
  /** Bytes per second. */
  speed: number;
  state: "active" | "waiting" | "done" | string;
}

/** What a download is doing now. */
export interface Details {
  id: number;
  status: string;
  downloaded: number;
  size: number | null;
  /** Bytes per second, all connections together. */
  speed: number;
  /** Empty for a download on a single connection. */
  connections: Connection[];
}

/** What the browser extension keeps (shared with the browsers through the engine). */
export interface ExtensionSettings {
  intercept: boolean;
  minSizeBytes: number;
  excludedHosts: string[];
  videoButton: boolean;
  buttonHiddenHosts: string[];
  autoRefresh: boolean;
}

/** Everything the window can ask for. Errors are rejected with plain text. */
export interface Api {
  status(): Promise<EngineStatus>;
  start(): Promise<EngineStatus>;
  list(): Promise<Row[]>;
  /** `files`: for a torrent, only these file numbers (from `torrentFiles`). */
  add(url: string, folder: string | null, queue?: number, files?: number[]): Promise<number>;
  /** What is inside a torrent; a magnet link can take up to 90 seconds. */
  torrentFiles(url: string): Promise<TorrentList>;
  grabStart(options: GrabOptions): Promise<void>;
  grabStatus(): Promise<GrabState>;
  grabStop(): Promise<void>;
  /** Adds the chosen files (each with the page that links to it as referrer). */
  grabDownload(files: GrabFile[], folder: string | null, queue: number | null): Promise<GrabAdded>;
  details(id: number): Promise<Details>;
  tools(): Promise<ToolInfo[]>;
  /** Chooses where a tool is (null: find it by itself); resolves with the new state. */
  setTool(name: string, path: string | null): Promise<ToolInfo[]>;
  /** Opens one of the tools' download pages in the browser. */
  openToolPage(url: string): Promise<void>;
  /** How long finished torrents are shared (ratio in hundredths; neither: not shared). */
  seeding(): Promise<Seeding>;
  setSeeding(seeding: Seeding): Promise<Seeding>;
  /** The browser whose sign-in yt-dlp uses for video pages (null: none). */
  scan(): Promise<ScanSetting>;
  setScan(mode: string, command: string): Promise<ScanSetting>;
  videoBrowser(): Promise<string | null>;
  setVideoBrowser(browser: string | null): Promise<string | null>;
  profiles(): Promise<SiteProfile[]>;
  /** Replaces the whole list; resolves with what was saved. */
  setProfiles(profiles: SiteProfile[]): Promise<SiteProfile[]>;
  rules(): Promise<Rule[]>;
  /** Replaces the whole list; resolves with what was saved. */
  setRules(rules: Rule[]): Promise<Rule[]>;
  queues(): Promise<Queue[]>;
  addQueue(name: string): Promise<Queue[]>;
  renameQueue(id: number, name: string): Promise<Queue[]>;
  removeQueue(id: number): Promise<Queue[]>;
  setQueue(id: number, change: QueueChange): Promise<Queue[]>;
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
  remove(id: number, deleteFile: boolean): Promise<void>;
  open(id: number): Promise<void>;
  show(id: number): Promise<void>;
  plan(): Promise<Plan>;
  setPlan(windows: PlanWindow[]): Promise<Plan>;
  setAfterAction(action: string | null, countdownSeconds?: number): Promise<Plan>;
  cancelAction(): Promise<Plan>;
  pickFolder(): Promise<string | null>;
  /** The text on the clipboard, or null. */
  clipboard(): Promise<string | null>;
  categories(): Promise<Category[]>;
  addCategory(name: string, extensions: string[], folder: string | null): Promise<Category[]>;
  removeCategory(id: number): Promise<Category[]>;
  config(): Promise<Config>;
  setConfig(change: ConfigChange): Promise<Config>;
  extensionSettings(): Promise<ExtensionSettings>;
  setExtensionSettings(settings: ExtensionSettings): Promise<ExtensionSettings>;
}

/** How finished downloads are scanned for viruses. */
export interface ScanSetting {
  mode: "off" | "defender" | "custom";
  command: string;
  /** Where Windows Defender's scanner is, if this computer has it. */
  defender: string | null;
}
