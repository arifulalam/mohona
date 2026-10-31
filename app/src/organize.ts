import { tr } from "./i18n";
import type { Category, Row } from "./types";

export type SortKey = "added" | "name" | "size" | "type" | "status";

export const SORTS: { id: SortKey; label: string }[] = [
  { id: "added", label: tr("Newest first") },
  { id: "name", label: tr("Name") },
  { id: "size", label: tr("Size") },
  { id: "type", label: tr("Category and type") },
  { id: "status", label: tr("Status") },
];

export const OTHER = tr("Other");

/** The engine's standard categories, listed so their names can be translated. */
export const STANDARD_CATEGORIES = [tr("Compressed"), tr("Documents"), tr("Music"), tr("Programs"), tr("Video")];

/** A file's extension in lower case, without the dot ("" when it has none). */
export function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : "";
}

/** The category whose file types include the file's, else "Other". */
export function categoryOf(name: string, categories: Category[]): string {
  const ext = extensionOf(name);
  if (!ext) return OTHER;
  return categories.find((c) => c.extensions.includes(ext))?.name ?? OTHER;
}

const ORDER: Record<string, number> = { downloading: 0, probing: 1, retrying: 2, queued: 3, paused: 4, failed: 5, canceled: 6, completed: 7 };

/** A sorted copy. Equal rows keep the newest-first order, so the result never jumps around. */
export function sortRows(rows: Row[], key: SortKey, categories: Category[]): Row[] {
  const newest = (a: Row, b: Row) => b.id - a.id;
  const byName = (a: Row, b: Row) => a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
  const compare: Record<SortKey, (a: Row, b: Row) => number> = {
    added: newest,
    name: byName,
    size: (a, b) => (b.size ?? b.downloaded) - (a.size ?? a.downloaded),
    type: (a, b) =>
      categoryOf(a.name, categories).localeCompare(categoryOf(b.name, categories)) ||
      extensionOf(a.name).localeCompare(extensionOf(b.name)) ||
      byName(a, b),
    status: (a, b) => (ORDER[a.status] ?? 9) - (ORDER[b.status] ?? 9),
  };
  return [...rows].sort((a, b) => compare[key](a, b) || newest(a, b));
}

/** How many rows each category has, plus "Other"; categories without rows are kept with 0. */
export function categoryCounts(rows: Row[], categories: Category[]): { name: string; count: number }[] {
  const counts = new Map<string, number>(categories.map((c) => [c.name, 0]));
  for (const row of rows) {
    const name = categoryOf(row.name, categories);
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  const out = [...counts].map(([name, count]) => ({ name, count }));
  return out.filter((c) => c.name !== OTHER || c.count > 0);
}
