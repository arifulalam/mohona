import type { AddDetails, Outcome } from "./protocol";

export const MENU_ID = "mohona-download";

/** What the browser tells about a click on the menu entry. */
export interface ClickInfo {
  linkUrl?: string;
  srcUrl?: string;
  pageUrl?: string;
  frameUrl?: string;
  mediaType?: string;
}

export type MenuResult =
  | { kind: "sent"; url: string }
  | { kind: "failed"; message: string }
  | { kind: "ignored"; reason: string };

export interface MenuDeps {
  userAgent: string;
  cookieHeader(url: string): Promise<string>;
  add(details: AddDetails): Promise<Outcome>;
}

const SCHEMES = new Set(["http:", "https:", "ftp:", "magnet:"]);

/**
 * The address to download: the link when the click was on one (even when it
 * wraps an image), otherwise the media's own source.
 */
export function targetOf(info: ClickInfo): string | null {
  const url = info.linkUrl || info.srcUrl;
  if (!url) return null;
  try {
    return SCHEMES.has(new URL(url).protocol) ? url : null;
  } catch {
    return null;
  }
}

/** A `data:` or `blob:` address on the clicked item (the link first), for files only the browser has. */
export function localTarget(info: ClickInfo): string | null {
  const url = info.linkUrl || info.srcUrl;
  return url && /^(data|blob):/i.test(url) ? url : null;
}

export async function downloadFromMenu(info: ClickInfo, deps: MenuDeps): Promise<MenuResult> {
  const url = targetOf(info);
  if (!url) return { kind: "ignored", reason: "that is not a downloadable address" };
  try {
    const cookies = await deps.cookieHeader(url).catch(() => "");
    const outcome = await deps.add({
      url,
      referrer: info.frameUrl || info.pageUrl || undefined,
      cookies: cookies || undefined,
      userAgent: deps.userAgent,
    });
    switch (outcome.kind) {
      case "added":
      case "streaming":
        return { kind: "sent", url };
      case "duplicate":
        return { kind: "failed", message: `Mohona already has it: ${outcome.message}` };
      case "error":
        return { kind: "failed", message: outcome.message };
    }
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
}
