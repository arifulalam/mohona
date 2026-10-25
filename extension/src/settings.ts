export interface Settings {
  /** Take over the browser's downloads. */
  intercept: boolean;
  /** Leave downloads smaller than this to the browser (0: take all). */
  minSizeBytes: number;
  /** Look for a fresh link by itself when a download's link has expired. */
  autoRefresh: boolean;
  /** Show a small "download" button on videos in pages. */
  videoButton: boolean;
  /** Sites (and their subdomains) where that button is not shown. */
  buttonHiddenHosts: string[];
  /** Never take downloads from these sites (and their subdomains). */
  excludedHosts: string[];
}

export const DEFAULTS: Settings = {
  intercept: true,
  autoRefresh: true,
  videoButton: true,
  buttonHiddenHosts: [],
  minSizeBytes: 0,
  excludedHosts: [],
};

/** Whatever was stored, made safe: wrong types fall back to the defaults. */
export function sanitize(raw: unknown): Settings {
  const r = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const hostList = (value: unknown, fallback: string[]): string[] =>
    Array.isArray(value)
      ? value
          .filter((h): h is string => typeof h === "string")
          .map((h) => h.trim().toLowerCase().replace(/^\*?\./, ""))
          .filter((h) => h.length > 0)
      : fallback;
  const hosts = hostList(r.excludedHosts, DEFAULTS.excludedHosts);
  return {
    intercept: typeof r.intercept === "boolean" ? r.intercept : DEFAULTS.intercept,
    videoButton: typeof r.videoButton === "boolean" ? r.videoButton : DEFAULTS.videoButton,
    buttonHiddenHosts: hostList(r.buttonHiddenHosts, DEFAULTS.buttonHiddenHosts),
    autoRefresh: typeof r.autoRefresh === "boolean" ? r.autoRefresh : DEFAULTS.autoRefresh,
    minSizeBytes:
      typeof r.minSizeBytes === "number" && Number.isFinite(r.minSizeBytes) && r.minSizeBytes >= 0
        ? Math.floor(r.minSizeBytes)
        : DEFAULTS.minSizeBytes,
    excludedHosts: hosts,
  };
}

/** `host` is `site` or one of its subdomains. */
export function hostMatches(host: string, site: string): boolean {
  const h = host.toLowerCase();
  return h === site || h.endsWith(`.${site}`);
}

export async function loadSettings(): Promise<Settings> {
  try {
    return sanitize(await chrome.storage.sync.get(null));
  } catch {
    return DEFAULTS;
  }
}

export async function saveSettings(patch: Partial<Settings>): Promise<Settings> {
  const next = sanitize({ ...(await loadSettings()), ...patch });
  await chrome.storage.sync.set(next);
  return next;
}
