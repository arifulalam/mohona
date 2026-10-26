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
}
