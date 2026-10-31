import { tr } from "./i18n";
import type { Row } from "./types";

export type Filter = "all" | "active" | "finished" | "problems";

export const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: tr("All downloads") },
  { id: "active", label: tr("In progress") },
  { id: "finished", label: tr("Finished") },
  { id: "problems", label: tr("Failed") },
];

const ACTIVE = new Set(["queued", "probing", "downloading", "paused", "retrying"]);

export function matches(row: Row, filter: Filter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "active":
      return ACTIVE.has(row.status);
    case "finished":
      return row.status === "completed";
    case "problems":
      return row.status === "failed" || row.status === "canceled";
  }
}

export function counts(rows: Row[]): Record<Filter, number> {
  const out: Record<Filter, number> = { all: rows.length, active: 0, finished: 0, problems: 0 };
  for (const row of rows) {
    for (const f of ["active", "finished", "problems"] as const) if (matches(row, f)) out[f]++;
  }
  return out;
}

/** What each button may do to a set of rows. */
export function allowed(rows: Row[]) {
  return {
    pause: rows.some((r) => ["queued", "probing", "downloading", "retrying"].includes(r.status)),
    resume: rows.some((r) => ["paused", "failed", "canceled"].includes(r.status)),
    cancel: rows.some((r) => ACTIVE.has(r.status)),
    remove: rows.length > 0,
  };
}
