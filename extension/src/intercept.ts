import type { AddDetails, Outcome } from "./protocol";
import { hostMatches, type Settings } from "./settings";

/** The parts of a browser download that matter here. */
export interface Item {
  id: number;
  url: string;
  finalUrl?: string;
  referrer?: string;
  fileSize?: number;
  incognito?: boolean;
}

export interface Deps {
  settings: Settings;
  userAgent: string;
  downloads: {
    pause(id: number): Promise<void>;
    resume(id: number): Promise<void>;
    cancel(id: number): Promise<void>;
    erase(id: number): Promise<void>;
  };
  /** The `Cookie` header value for an address ("" when there is none or it cannot be read). */
  cookieHeader(url: string): Promise<string>;
  add(details: AddDetails): Promise<Outcome>;
}

export type Result =
  | { kind: "handed"; url: string }
  | { kind: "skipped"; reason: string }
  | { kind: "kept"; reason: string };

const SCHEMES = new Set(["http:", "https:", "ftp:"]);

/** Should this download go to Mohona? Returns a reason when it should not. */
export function skipReason(item: Item, settings: Settings): string | null {
  if (!settings.intercept) return "interception is off";
  if (item.incognito) return "private window";
  const url = item.finalUrl || item.url;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return "not an address";
  }
  if (!SCHEMES.has(parsed.protocol)) return `${parsed.protocol} addresses stay in the browser`;
  if (settings.excludedHosts.some((site) => hostMatches(parsed.hostname, site))) {
    return `${parsed.hostname} is excluded`;
  }
  if (
    settings.minSizeBytes > 0 &&
    typeof item.fileSize === "number" &&
    item.fileSize > 0 &&
    item.fileSize < settings.minSizeBytes
  ) {
    return "smaller than the size limit";
  }
  return null;
}

/** `name=value; name2=value2` */
export function cookieHeaderOf(cookies: { name: string; value: string }[]): string {
  return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
}

async function quiet(action: () => Promise<void>): Promise<void> {
  try {
    await action();
  } catch {
    // The download may be finished or gone already; nothing to do about it.
  }
}

/**
 * Takes one browser download over. The rule is to fail open: the browser's
 * download is only cancelled after Mohona has accepted the address. While
 * Mohona is asked, the browser's download is paused, and it is resumed
 * whenever Mohona does not take it (not running, refused, already has it,
 * or anything unexpected).
 */
export async function intercept(item: Item, deps: Deps): Promise<Result> {
  const reason = skipReason(item, deps.settings);
  if (reason) return { kind: "skipped", reason };
  const url = item.finalUrl || item.url;
  await quiet(() => deps.downloads.pause(item.id));
  let taken = false;
  try {
    const cookies = await deps.cookieHeader(url).catch(() => "");
    const outcome = await deps.add({
      url,
      referrer: item.referrer || undefined,
      cookies: cookies || undefined,
      userAgent: deps.userAgent,
    });
    if (outcome.kind === "added") {
      taken = true;
      await quiet(() => deps.downloads.cancel(item.id));
      await quiet(() => deps.downloads.erase(item.id));
      return { kind: "handed", url };
    }
    return {
      kind: "kept",
      reason:
        outcome.kind === "duplicate"
          ? `Mohona already has it: ${outcome.message}`
          : outcome.kind === "error"
            ? outcome.message
            : "Mohona did not take it",
    };
  } catch (e) {
    return { kind: "kept", reason: e instanceof Error ? e.message : String(e) };
  } finally {
    if (!taken) await quiet(() => deps.downloads.resume(item.id));
  }
}
