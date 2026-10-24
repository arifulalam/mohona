import { cookieHeaderOf, intercept, type Item, type Result } from "./intercept";
import { addDownload, NativeClient, startEngine, status } from "./native";
import type { Reply, Request } from "./messages";
import { HOST_NAME } from "./protocol";
import { downloadFromMenu, MENU_ID, type ClickInfo } from "./menu";
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

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId !== MENU_ID) return;
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

async function handle(request: Request): Promise<Reply> {
  switch (request.kind) {
    case "status":
      return { kind: "status", status: await status(client), last };
    case "startEngine":
      return { kind: "outcome", outcome: await startEngine(client) };
    case "add":
      return { kind: "outcome", outcome: await addDownload(client, { url: request.url }) };
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
