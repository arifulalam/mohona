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
  add(url: string, folder: string | null, queue?: number): Promise<number>;
  details(id: number): Promise<Details>;
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
  categories(): Promise<Category[]>;
  addCategory(name: string, extensions: string[], folder: string | null): Promise<Category[]>;
  removeCategory(id: number): Promise<Category[]>;
  config(): Promise<Config>;
  setConfig(change: ConfigChange): Promise<Config>;
  extensionSettings(): Promise<ExtensionSettings>;
  setExtensionSettings(settings: ExtensionSettings): Promise<ExtensionSettings>;
}
