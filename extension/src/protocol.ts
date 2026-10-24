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
}

export type Question =
  | { type: "hello" }
  | { type: "startEngine" }
  | ({ type: "add" } & AddDetails);

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
  | { type: "engine"; running: boolean; started: boolean }
  | { type: "error"; code: string; message: string };

/** What the person sees. */
export type Outcome =
  | { kind: "added"; id: number }
  | { kind: "duplicate"; message: string }
  | { kind: "error"; code: string; message: string };
