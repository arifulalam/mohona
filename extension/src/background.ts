import { cookieHeaderOf, intercept, type Item, type Result } from "./intercept";
import { addDownload, NativeClient, startEngine, startStream, status } from "./native";
import { addMedia, classify, nameFromUrl, totalFromRange, type Media } from "./sniffer";
import type { Reply, Request } from "./messages";
import { HOST_NAME } from "./protocol";
import { downloadFromMenu, localTarget, MENU_ID, type ClickInfo } from "./menu";
import { saveLocal, sliceInPage, sourceFromBlob, sourceFromData, type SaveResult } from "./local";
import { adoptSettings, loadSettings, saveSettings } from "./settings";
import { reconcile } from "./settingsync";
import { linksInPage, nextToRefresh, refreshExpired, type RefreshResult } from "./refresh";
import type { DownloadRow } from "./protocol";
import { lookFor } from "./look";

const client = new NativeClient(() => chrome.runtime.connectNative(HOST_NAME));

/** What happened to the latest download, for the popup. */
let last: string | null = null;

function remember(result: Result): void {
  switch (result.kind) {
    case "handed":
      last = `Sent to Mohona: ${result.url}`;
      break;
    case "kept":
      last = `Left to the browser (${result.reason})`;
      break;
    case "skipped":
      break; // not worth mentioning
  }
}

async function cookieHeader(url: string): Promise<string> {
  return cookieHeaderOf(await chrome.cookies.getAll({ url }));
}

/** A short mark on the toolbar button: the person sees the result without opening anything. */
function badge(text: string, color: string): void {
  void chrome.action.setBadgeBackgroundColor({ color });
  void chrome.action.setBadgeText({ text });
  setTimeout(() => void chrome.action.setBadgeText({ text: "" }), 4000);
}

chrome.runtime.onInstalled.addListener(() => {
  // Menus are created once per install or update; the browser keeps them.
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_ID,
      title: "Download with Mohona",
      contexts: ["link", "image", "video", "audio"],
    });
  });
});

/** Saves a `data:` or `blob:` file the clicked page holds. */
async function saveFromPage(url: string, tab: chrome.tabs.Tab | undefined, frameId: number | undefined): Promise<SaveResult> {
  try {
    let source;
    if (/^data:/i.test(url)) {
      source = await sourceFromData(url);
    } else {
      if (tab?.id === undefined) return { kind: "failed", message: "no page to read the file from" };
      const tabId = tab.id;
      source = await sourceFromBlob(url, async (start, end) => {
        const [run] = await chrome.scripting.executeScript({
          target: { tabId, frameIds: [frameId ?? 0] },
          func: sliceInPage,
          args: [url, start, end],
        });
        return run?.result;
      });
    }
    return await saveLocal(client, source, tab?.title || "download");
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
}

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== MENU_ID) return;
  const local = localTarget(info as ClickInfo);
  if (local) {
    void saveFromPage(local, tab, info.frameId).then((result) => {
      if (result.kind === "saved") {
        last = `Saved by Mohona: ${result.path}`;
        badge("✓", "#2e7d32");
      } else {
        last = `Not saved: ${result.message}`;
        badge("!", "#c62828");
      }
    });
    return;
  }
  void (async () => {
    const result = await downloadFromMenu(info as ClickInfo, {
      userAgent: navigator.userAgent,
      cookieHeader,
      add: (details) => addDownload(client, details),
    });
    if (result.kind === "sent") {
      last = `Sent to Mohona: ${result.url}`;
      badge("✓", "#2e7d32");
    } else if (result.kind === "failed") {
      last = `Not sent: ${result.message}`;
      badge("!", "#c62828");
    }
  })();
});

chrome.downloads.onCreated.addListener((item) => {
  void (async () => {
    const result = await intercept(item as Item, {
      settings: await loadSettings(),
      userAgent: navigator.userAgent,
      downloads: {
        pause: (id) => chrome.downloads.pause(id),
        resume: (id) => chrome.downloads.resume(id),
        cancel: (id) => chrome.downloads.cancel(id),
        erase: async (id) => void (await chrome.downloads.erase({ id })),
      },
      cookieHeader,
      add: (details) => addDownload(client, details),
    });
    remember(result);
  })();
});

// --- media found on pages -------------------------------------------------

// The worker may be stopped at any time, so the list lives in session storage.
const key = (tabId: number) => `media:${tabId}`;
let queue: Promise<void> = Promise.resolve();

async function mediaOf(tabId: number): Promise<Media[]> {
  const stored = await chrome.storage.session.get(key(tabId));
  const list = stored[key(tabId)];
  return Array.isArray(list) ? (list as Media[]) : [];
}

function showCount(tabId: number, count: number): void {
  void chrome.action.setBadgeBackgroundColor({ tabId, color: "#1565c0" });
  void chrome.action.setBadgeText({ tabId, text: count > 0 ? String(count) : "" });
}

/** One change at a time, so two quick responses do not overwrite each other. */
function changeMedia(tabId: number, change: (list: Media[]) => Media[]): void {
  queue = queue
    .then(async () => {
      const list = change(await mediaOf(tabId));
      await chrome.storage.session.set({ [key(tabId)]: list });
      showCount(tabId, list.length);
    })
    .catch(() => undefined);
}

chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    if (details.tabId >= 0) changeMedia(details.tabId, () => []); // a new page
  },
  { urls: ["<all_urls>"], types: ["main_frame"] },
);

chrome.webRequest.onHeadersReceived.addListener(
  (details) => {
    if (details.tabId < 0) return;
    const header = (name: string) =>
      details.responseHeaders?.find((h) => h.name.toLowerCase() === name)?.value;
    const length = Number(header("content-length"));
    const size = totalFromRange(header("content-range")) ?? (Number.isFinite(length) && length > 0 ? length : undefined);
    const contentType = header("content-type");
    const kind = classify(details.url, contentType, size);
    if (!kind) return;
    const page = (details as { documentUrl?: string }).documentUrl ?? details.initiator;
    changeMedia(details.tabId, (list) =>
      addMedia(list, {
        url: details.url,
        kind,
        name: nameFromUrl(details.url),
        contentType,
        size,
        referrer: page ? (page.endsWith("/") || page.includes("/", 8) ? page : `${page}/`) : undefined,
      }),
    );
  },
  { urls: ["<all_urls>"], types: ["media", "xmlhttprequest", "other"] },
  ["responseHeaders"],
);

chrome.tabs.onRemoved.addListener((tabId) => void chrome.storage.session.remove(key(tabId)));

async function downloadMedia(media: Media) {
  const cookies = await cookieHeader(media.url).catch(() => "");
  const details = {
    url: media.url,
    referrer: media.referrer,
    cookies: cookies || undefined,
    userAgent: navigator.userAgent,
    name: media.name,
  };
  return media.kind === "file"
    ? addDownload(client, { ...details, name: undefined })
    : startStream(client, details);
}

// --- expired links ---------------------------------------------------------

/** How many times each download's link was tried in this browser session. */
const tries = new Map<number, number>();

async function downloadsList(): Promise<DownloadRow[]> {
  const answer = await client.request({ type: "list" }, 10_000);
  return answer.type === "downloads" ? answer.downloads : [];
}

/** Opens the page in a background tab, waits for it to load, and reads the addresses on it. */
async function collectLinks(page: string): Promise<string[]> {
  const tab = await chrome.tabs.create({ url: page, active: false });
  const tabId = tab.id;
  if (tabId === undefined) throw new Error("could not open the page");
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => done(() => reject(new Error("the page took too long to load"))), 25_000);
      const listener = (id: number, change: chrome.tabs.TabChangeInfo) => {
        if (id === tabId && change.status === "complete") done(resolve);
      };
      const done = (finish: () => void) => {
        clearTimeout(timer);
        chrome.tabs.onUpdated.removeListener(listener);
        finish();
      };
      chrome.tabs.onUpdated.addListener(listener);
    });
    // Players and download buttons often build their links a moment after loading.
    await new Promise((r) => setTimeout(r, 1500));
    const [run] = await chrome.scripting.executeScript({ target: { tabId }, func: linksInPage });
    const found = new Set<string>(Array.isArray(run?.result) ? (run.result as string[]) : []);
    for (const media of await mediaOf(tabId)) found.add(media.url);
    return [...found];
  } finally {
    await chrome.tabs.remove(tabId).catch(() => undefined);
  }
}

async function refreshOne(download: DownloadRow): Promise<RefreshResult> {
  tries.set(download.id, (tries.get(download.id) ?? 0) + 1);
  const result = await refreshExpired(download, {
    collect: collectLinks,
    cookieHeader,
    userAgent: navigator.userAgent,
    refresh: async (id, url, cookies, referrer, userAgent) => {
      const answer = await client.request(
        { type: "refresh", id, url, cookies: cookies || undefined, referrer, userAgent },
        30_000,
      );
      if (answer.type === "refreshed") return null;
      return answer.type === "error" ? answer.message : "Mohona gave an unexpected answer";
    },
  });
  last =
    result.kind === "refreshed"
      ? `Continued from a fresh link: ${download.filename ?? download.url}`
      : `Could not refresh the link of ${download.filename ?? download.url}: ${describeRefresh(result)}`;
  return result;
}

function describeRefresh(result: RefreshResult): string {
  switch (result.kind) {
    case "no_page":
      return "the page it came from is not known";
    case "no_link":
      return "no matching link was found on the page (open it and download again)";
    case "refused":
    case "failed":
      return result.message;
    case "refreshed":
      return "done";
  }
}

/** Compares the settings with Mohona's copy and updates the older one. Does nothing when Mohona is not running. */
async function syncSettings(): Promise<void> {
  try {
    const answer = await client.request({ type: "getSettings" }, 5_000);
    if (answer.type !== "settings") return;
    const plan = reconcile(await loadSettings(), answer.value);
    if (plan.apply) await adoptSettings(plan.apply);
    if (plan.push) await client.request({ type: "setSettings", value: JSON.stringify(plan.push) }, 5_000);
  } catch {
    // Next time.
  }
}
void syncSettings();

chrome.alarms.create("mohona-refresh", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name !== "mohona-refresh") return;
  void (async () => {
    await syncSettings();
    if (!(await loadSettings()).autoRefresh) return;
    // Only when the engine is already running: this never starts it.
    const rows = await downloadsList().catch(() => [] as DownloadRow[]);
    const next = nextToRefresh(rows, tries);
    if (next) await refreshOne(next);
  })();
});

async function handle(request: Request, sender: chrome.runtime.MessageSender): Promise<Reply> {
  switch (request.kind) {
    case "status":
      return { kind: "status", status: await status(client), last };
    case "startEngine":
      return { kind: "outcome", outcome: await startEngine(client) };
    case "add":
      return { kind: "outcome", outcome: await addDownload(client, { url: request.url }) };
    case "media":
      return { kind: "media", media: await mediaOf(request.tabId) };
    case "pageMedia": {
      // From the button on a video: what the tab has shown, plus the video's own address.
      const tabId = sender.tab?.id;
      const found = tabId === undefined ? [] : await mediaOf(tabId);
      if (request.direct && !found.some((m) => m.url === request.direct)) {
        found.unshift({
          url: request.direct,
          kind: "file",
          name: nameFromUrl(request.direct),
          referrer: request.page,
        });
      }
      // Streams (which carry video and sound together) before plain files.
      found.sort((a, b) => Number(a.kind === "file") - Number(b.kind === "file"));
      return { kind: "media", media: found };
    }
    case "downloadMedia": {
      const outcome = await downloadMedia(request.media);
      last = outcome.kind === "error" ? `Not sent: ${outcome.message}` : `Sent to Mohona: ${request.media.url}`;
      return { kind: "outcome", outcome };
    }
    case "saveLocal": {
      const tab = await chrome.tabs.get(request.tabId);
      return { kind: "saved", result: await saveFromPage(request.url, tab, 0) };
    }
    case "expired": {
      const rows = await downloadsList().catch(() => [] as DownloadRow[]);
      return { kind: "expired", downloads: rows.filter((r) => r.expired && r.status === "failed") };
    }
    case "downloads":
      return { kind: "downloads", downloads: await downloadsList().catch(() => [] as DownloadRow[]) };
    case "refreshDownload": {
      const row = (await downloadsList()).find((r) => r.id === request.id);
      if (!row) return { kind: "refreshed", result: { kind: "failed", message: "that download is gone" } };
      tries.delete(row.id);
      return { kind: "refreshed", result: await refreshOne(row) };
    }
    case "openFile": {
      try {
        const answer = await client.request({ type: request.reveal ? "reveal" : "open", id: request.id }, 10_000);
        return { kind: "opened", error: answer.type === "error" ? answer.message : null };
      } catch (e) {
        return { kind: "opened", error: (e as Error).message };
      }
    }
    case "look": {
      const stored = (await chrome.storage.local.get(["mohona.theme", "mohona.language"]).catch(() => ({}))) as Record<string, unknown>;
      const text = (v: unknown) => (typeof v === "string" ? v : null);
      return { kind: "look", look: lookFor(text(stored["mohona.theme"]), text(stored["mohona.language"]), navigator.languages ?? []) };
    }
    case "getSettings":
      return { kind: "settings", settings: await loadSettings() };
    case "setSettings": {
      const settings = await saveSettings(request.settings);
      void syncSettings();
      return { kind: "settings", settings };
    }
  }
}

chrome.runtime.onMessage.addListener((message: Request, _sender, sendResponse) => {
  void handle(message, _sender).then(sendResponse);
  return true; // the answer comes later
});
