import { cookieHeaderOf, intercept, type Item, type Result } from "./intercept";
import { addDownload, NativeClient, startEngine, startStream, status } from "./native";
import { addMedia, classify, nameFromUrl, totalFromRange, type Media } from "./sniffer";
import type { Reply, Request } from "./messages";
import { HOST_NAME } from "./protocol";
import { downloadFromMenu, localTarget, MENU_ID, type ClickInfo } from "./menu";
import { saveLocal, sliceInPage, sourceFromBlob, sourceFromData, type SaveResult } from "./local";
import { loadSettings, saveSettings } from "./settings";

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

async function handle(request: Request): Promise<Reply> {
  switch (request.kind) {
    case "status":
      return { kind: "status", status: await status(client), last };
    case "startEngine":
      return { kind: "outcome", outcome: await startEngine(client) };
    case "add":
      return { kind: "outcome", outcome: await addDownload(client, { url: request.url }) };
    case "media":
      return { kind: "media", media: await mediaOf(request.tabId) };
    case "downloadMedia": {
      const outcome = await downloadMedia(request.media);
      last = outcome.kind === "error" ? `Not sent: ${outcome.message}` : `Sent to Mohona: ${request.media.url}`;
      return { kind: "outcome", outcome };
    }
    case "saveLocal": {
      const tab = await chrome.tabs.get(request.tabId);
      return { kind: "saved", result: await saveFromPage(request.url, tab, 0) };
    }
    case "getSettings":
      return { kind: "settings", settings: await loadSettings() };
    case "setSettings":
      return { kind: "settings", settings: await saveSettings(request.settings) };
  }
}

chrome.runtime.onMessage.addListener((message: Request, _sender, sendResponse) => {
  void handle(message).then(sendResponse);
  return true; // the answer comes later
});
