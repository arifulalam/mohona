export type Unit = "KB" | "MB";

const FACTOR: Record<Unit, number> = { KB: 1024, MB: 1024 * 1024 };

/** The speed limit in bytes per second from what was typed; 0 means no limit. null: not a number. */
export function parseLimit(text: string, unit: Unit): number | null {
  const t = text.trim().replace(",", ".");
  if (t === "") return 0;
  if (!/^\d+(\.\d+)?$/.test(t)) return null;
  return Math.round(Number(t) * FACTOR[unit]);
}

/** Splits bytes per second into a number to show and its unit (MB when it is a whole number of them). */
export function splitLimit(bytes: number | null): { text: string; unit: Unit } {
  if (!bytes) return { text: "", unit: "MB" };
  if (bytes >= FACTOR.MB && bytes % FACTOR.MB === 0) return { text: String(bytes / FACTOR.MB), unit: "MB" };
  return { text: String(Math.round((bytes / FACTOR.KB) * 10) / 10), unit: "KB" };
}

/** The minimum size to take over, in whole MB as typed; 0 or empty takes all. */
export function parseMegabytes(text: string): number | null {
  const t = text.trim();
  if (t === "") return 0;
  if (!/^\d+$/.test(t)) return null;
  return Number(t) * 1024 * 1024;
}

/** One site per line. */
export function parseHosts(text: string): string[] {
  return text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
}
