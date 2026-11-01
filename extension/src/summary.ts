import type { DownloadRow } from "./protocol";

const RUNNING = new Set(["downloading", "probing", "retrying"]);

export interface Summary {
  running: number;
  waiting: number;
  finished: number;
  failed: number;
}

/** The numbers shown in the popup's four boxes. Paused downloads count as waiting. */
export function summarize(rows: DownloadRow[]): Summary {
  const out: Summary = { running: 0, waiting: 0, finished: 0, failed: 0 };
  for (const row of rows) {
    if (RUNNING.has(row.status)) out.running++;
    else if (row.status === "queued" || row.status === "paused") out.waiting++;
    else if (row.status === "completed") out.finished++;
    else if (row.status === "failed" || row.status === "canceled") out.failed++;
  }
  return out;
}

export const isRunning = (row: DownloadRow): boolean => RUNNING.has(row.status);

export function bytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = n;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export const speed = (perSecond: number): string => `${bytes(Math.max(0, Math.round(perSecond)))}/s`;

export function eta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 60) return `${Math.ceil(seconds)} s`;
  const m = Math.floor(seconds / 60);
  if (m < 60) return `${m} min ${Math.round(seconds % 60)} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

/** Bytes per second for each running download, from two lists some time apart. */
export function speeds(before: Map<number, number>, rows: DownloadRow[], seconds: number): Map<number, number> {
  const out = new Map<number, number>();
  if (seconds <= 0) return out;
  for (const row of rows) {
    const old = before.get(row.id);
    if (old !== undefined && isRunning(row) && row.downloaded >= old) out.set(row.id, (row.downloaded - old) / seconds);
  }
  return out;
}

export function nameOf(row: DownloadRow): string {
  if (row.filename) return row.filename;
  try {
    const last = new URL(row.url).pathname.split("/").filter(Boolean).pop();
    return last ? decodeURIComponent(last) : new URL(row.url).host;
  } catch {
    return row.url;
  }
}

/** Shortens every long web address in a message to its start and end, so it fits. */
export function brief(text: string, max = 56): string {
  return text.replace(/https?:\/\/\S+/g, (url) =>
    url.length <= max ? url : `${url.slice(0, Math.ceil(max * 0.6))}…${url.slice(-Math.floor(max * 0.3))}`,
  );
}
