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
  /** The address is a web page whose video is wanted: the engine asks yt-dlp, and no video is an error. */
  video?: boolean;
  /** The link was downloaded before: replace that download (its file is gone, or wanted again). */
  again?: boolean;
  /** For a video page: "best", a height such as "720", or "audio". */
  quality?: string;
  /** The video format: mp4, mkv, webm, mov or avi. */
  container?: string;
  /** The sound format, when `quality` is "audio": mp3, m4a, opus, ogg, flac, wav or aac. */
  audio?: string;
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
  | { type: "duplicate"; message: string; id?: number | null; status?: string | null; exists?: boolean }
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
  | { kind: "duplicate"; message: string; id?: number; status?: string; exists?: boolean }
  | { kind: "streaming"; job: number; folder: string }
  | { kind: "error"; code: string; message: string };
