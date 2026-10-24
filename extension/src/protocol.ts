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

export type Question =
  | { type: "hello" }
  | { type: "startEngine" }
  | ({ type: "add" } & AddDetails)
  | ({ type: "stream" } & AddDetails)
  | { type: "saveStart"; name: string; mime?: string }
  | { type: "savePiece"; transfer: number; index: number; data: string }
  | { type: "saveEnd"; transfer: number }
  | { type: "saveAbort"; transfer: number };

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
