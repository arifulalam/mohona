import type { Outcome } from "./protocol";
import type { Status } from "./native";

/** Popup (and later pages) to the background script. */
export type Request =
  | { kind: "status" }
  | { kind: "startEngine" }
  | { kind: "add"; url: string };

export type Reply =
  | { kind: "status"; status: Status }
  | { kind: "outcome"; outcome: Outcome | null };
