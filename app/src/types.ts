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
}

export interface ConfigChange {
  downloadFolder?: string;
  /** Bytes per second; 0 removes the limit. */
  speedLimit?: number;
  maxActive?: number;
  notifications?: boolean;
  keepInTray?: boolean;
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
  add(url: string, folder: string | null): Promise<number>;
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
  remove(id: number, deleteFile: boolean): Promise<void>;
  open(id: number): Promise<void>;
  show(id: number): Promise<void>;
  config(): Promise<Config>;
  setConfig(change: ConfigChange): Promise<Config>;
  extensionSettings(): Promise<ExtensionSettings>;
  setExtensionSettings(settings: ExtensionSettings): Promise<ExtensionSettings>;
}
