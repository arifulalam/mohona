import { cookieHeaderOf, intercept, type Item, type Result } from "./intercept";
import { addDownload, NativeClient, startEngine, status } from "./native";
import type { Reply, Request } from "./messages";
import { HOST_NAME } from "./protocol";
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
