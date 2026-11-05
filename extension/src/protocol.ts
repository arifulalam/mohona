// The messages between the extension and `mohona-host`. See
// docs/commands.md ("mohona-host") for the other side.

export const PROTOCOL = 1;
export const HOST_NAME = "org.mohona.host";

export interface AddDetails {
  url: string;
  referrer?: string;
  cookies?: string;
  userAgent?: string;
  headers?: { name: string; value: string }[];
  /** A file name to use (streams only). */
  name?: string;
}

/** A download in Mohona's list. */
export interface DownloadRow {
  id: number;
  url: string;
  filename: string | null;
  status: string;
  downloaded: number;
  size: number | null;
  error: string | null;
  path: string | null;
  /** The page the link was on (known for expired links). */
  page: string | null;
  /** The server refused the link (403 or 410): it probably expired. */
  expired: boolean;
}

export type Question =
  | { type: "hello" }
  | { type: "startEngine" }
  | ({ type: "add" } & AddDetails)
  | ({ type: "stream" } & AddDetails)
  | { type: "list" }
  | { type: "getSettings" }
  | { type: "setSettings"; value: string }
  | { type: "refresh"; id: number; url: string; cookies?: string; referrer?: string; userAgent?: string }
  | { type: "saveStart"; name: string; mime?: string }
  | { type: "savePiece"; transfer: number; index: number; data: string }
  | { type: "saveEnd"; transfer: number }
  | { type: "saveAbort"; transfer: number }
  | { type: "open"; id: number }
  | { type: "reveal"; id: number };

export type Answer =
  | {
      type: "hello";
      protocol: number;
      version: string;
      engineRunning: boolean;
      engineVersion: string | null;
    }
  | { type: "added"; id: number }
  | { type: "duplicate"; message: string }
  | { type: "streamStarted"; job: number; folder: string }
  | { type: "downloads"; downloads: DownloadRow[] }
  | { type: "refreshed"; id: number }
  | { type: "opened" }
  | { type: "settings"; value: string | null }
  | { type: "settingsSaved" }
  | { type: "saving"; transfer: number }
  | { type: "piece"; index: number }
  | { type: "saved"; path: string; name: string }
  | { type: "aborted" }
  | { type: "engine"; running: boolean; started: boolean }
  | { type: "error"; code: string; message: string };

/** What the person sees. */
export type Outcome =
  | { kind: "added"; id: number }
  | { kind: "duplicate"; message: string }
  | { kind: "streaming"; job: number; folder: string }
  | { kind: "error"; code: string; message: string };
