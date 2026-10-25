import type { Reply, Request } from "./messages";
import type { Outcome } from "./protocol";

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

function ask(request: Request): Promise<Reply> {
  return chrome.runtime.sendMessage(request) as Promise<Reply>;
}

function describe(outcome: Outcome | null): string {
  if (!outcome) return "";
  switch (outcome.kind) {
    case "added":
      return "Added to Mohona.";
    case "streaming":
      return `Stream download started. It will be saved in ${outcome.folder}.`;
    case "duplicate":
      return `Not added: ${outcome.message}`;
    case "error":
      return `Could not add: ${outcome.message}`;
  }
}

async function refresh(): Promise<void> {
  const reply = await ask({ kind: "status" });
  if (reply.kind !== "status") return;
  $("last").textContent = reply.last ?? "";
  const s = reply.status;
  const text = $("status");
  const start = $<HTMLButtonElement>("start");
  start.hidden = s.state !== "engine_stopped";
  $<HTMLButtonElement>("add").disabled = s.state === "host_missing" || s.state === "incompatible";
  switch (s.state) {
    case "ready":
      text.textContent = `Mohona ${s.version} is running.`;
      break;
    case "engine_stopped":
      text.textContent = "Mohona is not running.";
      break;
    case "host_missing":
      text.textContent = `Mohona is not connected: ${s.message}. Run "mohona-host install".`;
      break;
    case "incompatible":
      text.textContent = s.message;
      break;
  }
}

$("start").addEventListener("click", async () => {
  const reply = await ask({ kind: "startEngine" });
  if (reply.kind === "outcome") $("result").textContent = describe(reply.outcome);
  await refresh();
});

$<HTMLFormElement>("form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const url = $<HTMLInputElement>("url").value.trim();
  if (!url) return;
  const reply = await ask({ kind: "add", url });
  if (reply.kind === "outcome") $("result").textContent = describe(reply.outcome);
  await refresh();
});

$<HTMLInputElement>("videoButton").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { videoButton: (event.target as HTMLInputElement).checked } });
});
$<HTMLInputElement>("autoRefresh").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { autoRefresh: (event.target as HTMLInputElement).checked } });
});

$<HTMLInputElement>("intercept").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { intercept: (event.target as HTMLInputElement).checked } });
});

function size(bytes: number | undefined): string {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

async function showExpired(): Promise<void> {
  const reply = await ask({ kind: "expired" });
  if (reply.kind !== "expired") return;
  $("expiredBox").hidden = reply.downloads.length === 0;
  const list = $("expired");
  list.replaceChildren();
  for (const download of reply.downloads) {
    const row = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = download.filename ?? download.url;
    label.title = download.url;
    const button = document.createElement("button");
    button.textContent = "Refresh link";
    button.addEventListener("click", async () => {
      button.disabled = true;
      button.textContent = "Looking…";
      await ask({ kind: "refreshDownload", id: download.id });
      await refresh();
      await showExpired();
    });
    row.append(label, button);
    list.append(row);
  }
}

async function showMedia(): Promise<void> {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const list = $("media");
  list.replaceChildren();
  if (tab?.id === undefined) return;
  const reply = await ask({ kind: "media", tabId: tab.id });
  if (reply.kind !== "media") return;
  if (reply.media.length === 0) {
    const none = document.createElement("li");
    none.textContent = "Nothing found yet. Play the video, then open this again.";
    list.append(none);
  }
  for (const media of reply.media) {
    const row = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = media.name;
    label.title = media.url;
    const info = document.createElement("small");
    info.textContent = [media.kind === "file" ? "file" : media.kind.toUpperCase(), size(media.size)]
      .filter(Boolean)
      .join(" ");
    const button = document.createElement("button");
    button.textContent = "Download";
    button.addEventListener("click", async () => {
      button.disabled = true;
      const answer = await ask({ kind: "downloadMedia", media });
      if (answer.kind === "outcome") $("result").textContent = describe(answer.outcome);
      button.disabled = false;
      await refresh();
    });
    row.append(label, info, button);
    list.append(row);
  }
}

void (async () => {
  const reply = await ask({ kind: "getSettings" });
  if (reply.kind === "settings") {
    $<HTMLInputElement>("intercept").checked = reply.settings.intercept;
    $<HTMLInputElement>("autoRefresh").checked = reply.settings.autoRefresh;
    $<HTMLInputElement>("videoButton").checked = reply.settings.videoButton;
  }
  await refresh();
  await showExpired();
  await showMedia();
})();
