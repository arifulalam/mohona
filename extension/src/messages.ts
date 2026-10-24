import type { Outcome } from "./protocol";
import type { Status } from "./native";
import type { Settings } from "./settings";

/** Popup (and later pages) to the background script. */
export type Request =
  | { kind: "status" }
  | { kind: "startEngine" }
  | { kind: "add"; url: string }
  | { kind: "getSettings" }
  | { kind: "setSettings"; settings: Partial<Settings> };

export type Reply =
  | { kind: "status"; status: Status; last: string | null }
  | { kind: "settings"; settings: Settings }
  | { kind: "outcome"; outcome: Outcome | null };
