import type { DownloadRow, Outcome } from "./protocol";
import type { RefreshResult } from "./refresh";
import type { Status } from "./native";
import type { Media } from "./sniffer";
import type { Settings } from "./settings";
import type { SaveResult } from "./local";
import type { Look } from "./look";

/** Popup (and later pages) to the background script. */
export type Request =
  | { kind: "status" }
  | { kind: "startEngine" }
  | { kind: "add"; url: string }
  | { kind: "media"; tabId: number }
  | { kind: "downloadMedia"; media: Media }
  | { kind: "saveLocal"; url: string; tabId: number }
  | { kind: "pageMedia"; direct?: string; page?: string }
  | { kind: "expired" }
  | { kind: "downloads" }
  | { kind: "refreshDownload"; id: number }
  /** Open a finished download with its program, or show it in its folder. */
  | { kind: "openFile"; id: number; reveal: boolean }
  | { kind: "look" }
  | { kind: "getSettings" }
  | { kind: "setSettings"; settings: Partial<Settings> };

export type Reply =
  | { kind: "status"; status: Status; last: string | null }
  | { kind: "media"; media: Media[] }
  | { kind: "saved"; result: SaveResult }
  | { kind: "expired"; downloads: DownloadRow[] }
  | { kind: "downloads"; downloads: DownloadRow[] }
  | { kind: "refreshed"; result: RefreshResult }
  | { kind: "opened"; error: string | null }
  | { kind: "look"; look: Look }
  | { kind: "settings"; settings: Settings }
  | { kind: "outcome"; outcome: Outcome | null };
