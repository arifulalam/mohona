// Expired links. A download whose link was refused (403 or 410) is usually
// fine again with a fresh link from the page it came from: sites sign their
// links and sign them again every time the page loads. The page is opened in
// a background tab, the links on it are collected, the one that looks like
// the old link is chosen, and the engine checks it is the same file before
// it continues the download.

import type { DownloadRow } from "./protocol";

/** How alike a candidate is to the old link; higher is better, 0 means "not it". */
export function score(old: string, candidate: string): number {
  let a: URL;
  let b: URL;
  try {
    a = new URL(old);
    b = new URL(candidate);
  } catch {
    return 0;
  }
  if (a.href === b.href) return 0; // the link that already failed
  if (b.protocol !== "http:" && b.protocol !== "https:") return 0;
  const last = (u: URL) => {
    const part = u.pathname.split("/").filter(Boolean).pop() ?? "";
    try {
      return decodeURIComponent(part).toLowerCase();
    } catch {
      return part.toLowerCase();
    }
  };
  // The usual case: the same file on the same site, with a new signature.
  if (a.origin === b.origin && a.pathname === b.pathname) return 3;
  // The same file name on another host (a different mirror or CDN node).
  if (last(a) !== "" && last(a) === last(b)) return 2;
  return 0;
}

/** The best fresh link among those found on the page, or null. */
export function pickLink(old: DownloadRow, candidates: string[]): string | null {
  let best: { link: string; score: number } | null = null;
  const wanted = old.filename ? { url: new URL(old.url).origin + "/" + old.filename } : undefined;
  for (const link of candidates) {
    let s = score(old.url, link);
    if (s === 0 && wanted && old.filename) {
      // The old address had no usable name, but the page links to the file by name.
      try {
        const name = decodeURIComponent(new URL(link).pathname.split("/").pop() ?? "").toLowerCase();
        if (name === old.filename.toLowerCase() && link !== old.url) s = 1;
      } catch {
        s = 0;
      }
    }
    if (s > 0 && (!best || s >= best.score)) best = { link, score: s };
  }
  return best?.link ?? null;
}

export type RefreshResult =
  | { kind: "refreshed" }
  | { kind: "no_page" }
  | { kind: "no_link" }
  | { kind: "refused"; message: string }
  | { kind: "failed"; message: string };

export interface RefreshDeps {
  /** Opens the page in the background and returns the links on it. */
  collect(page: string): Promise<string[]>;
  cookieHeader(url: string): Promise<string>;
  userAgent: string;
  /** Asks the engine to continue the download from `url`. Returns an error message, or null when it took it. */
  refresh(id: number, url: string, cookies: string, referrer: string, userAgent: string): Promise<string | null>;
}

export async function refreshExpired(download: DownloadRow, deps: RefreshDeps): Promise<RefreshResult> {
  if (!download.page) return { kind: "no_page" };
  try {
    const link = pickLink(download, await deps.collect(download.page));
    if (!link) return { kind: "no_link" };
    const cookies = await deps.cookieHeader(link).catch(() => "");
    const problem = await deps.refresh(download.id, link, cookies, download.page, deps.userAgent);
    return problem === null ? { kind: "refreshed" } : { kind: "refused", message: problem };
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
}

/** Gives up on a download after this many tries (per browser session). */
export const MAX_TRIES = 2;

/** The one to work on next: expired, with a page, not tried too often. */
export function nextToRefresh(rows: DownloadRow[], tries: Map<number, number>): DownloadRow | undefined {
  return rows.find((r) => r.expired && r.status === "failed" && !!r.page && (tries.get(r.id) ?? 0) < MAX_TRIES);
}

/** The function that runs in the opened page: every address on it that could be the file. */
export function linksInPage(): string[] {
  const found: string[] = [];
  const add = (value: string | null | undefined) => {
    if (!value) return;
    try {
      found.push(new URL(value, document.baseURI).href);
    } catch {
      // not an address
    }
  };
  document.querySelectorAll("a[href]").forEach((a) => add((a as HTMLAnchorElement).getAttribute("href")));
  document.querySelectorAll("video[src], audio[src], source[src], embed[src]").forEach((e) => add(e.getAttribute("src")));
  document.querySelectorAll("a[download]").forEach((a) => add(a.getAttribute("href")));
  return [...new Set(found)];
}
