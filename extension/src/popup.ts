import type { Reply, Request } from "./messages";
import type { Outcome } from "./protocol";
import type { Settings } from "./settings";
import { label as languageLabel, LANGUAGES, loadLanguage, saveLanguage, translator, type Language, type Translate } from "./i18n";
import { ALL_THEMES, applyTheme, loadTheme, saveTheme } from "./popuptheme";
import type { DownloadRow } from "./protocol";
import { collectLinks, filesOf, GROUPS, type PageFile, type RawLink } from "./pagelinks";
import { bytes, eta, isRunning, nameOf, speed, speeds, summarize } from "./summary";

let t: Translate = translator({});

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

function ask(request: Request): Promise<Reply> {
  return chrome.runtime.sendMessage(request) as Promise<Reply>;
}

function setResult(text: string): void {
  const el = $("result");
  el.textContent = text;
  el.scrollTop = 0;
}

function describe(outcome: Outcome | null): string {
  if (!outcome) return "";
  switch (outcome.kind) {
    case "added":
      return t("Added to Mohona.");
    case "streaming":
      return t("Stream download started. It will be saved in {folder}.", { folder: outcome.folder });
    case "duplicate":
      return t("Not added: {message}", { message: outcome.message });
    case "error":
      return t("Could not add: {message}", { message: outcome.message });
  }
}

async function refresh(): Promise<void> {
  const reply = await ask({ kind: "status" });
  if (reply.kind !== "status") return;
  {
    const last = reply.last ?? "";
    $("last").textContent = last;
  }
  const s = reply.status;
  const text = $("status");
  const dot = $("dot");
  dot.className = `dot ${s.state === "ready" ? "ready" : s.state === "engine_stopped" ? "stopped" : "broken"}`;
  const start = $<HTMLButtonElement>("start");
  start.hidden = s.state !== "engine_stopped";
  $<HTMLButtonElement>("add").disabled = s.state === "host_missing" || s.state === "incompatible";
  switch (s.state) {
    case "ready":
      text.textContent = t("Mohona {version} is running", { version: s.version });
      break;
    case "engine_stopped":
      text.textContent = t("Mohona is not running");
      break;
    case "host_missing":
      text.textContent = t("Mohona is not connected: {message}. Run \"mohona-host install\".", { message: s.message });
      break;
    case "incompatible":
      text.textContent = s.message;
      break;
  }
}

$("start").addEventListener("click", async () => {
  const reply = await ask({ kind: "startEngine" });
  if (reply.kind === "outcome") setResult(describe(reply.outcome));
  await refresh();
});

$<HTMLFormElement>("form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const url = $<HTMLInputElement>("url").value.trim();
  if (!url) return;
  const reply = await ask({ kind: "add", url });
  if (reply.kind === "outcome") setResult(describe(reply.outcome));
  await refresh();
});

$<HTMLInputElement>("videoButton").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { videoButton: (event.target as HTMLInputElement).checked } });
});
$<HTMLInputElement>("autoRefresh").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { autoRefresh: (event.target as HTMLInputElement).checked } });
});

$<HTMLButtonElement>("saveMore").addEventListener("click", async () => {
  const mb = Number($<HTMLInputElement>("minMb").value);
  const excludedHosts = $<HTMLTextAreaElement>("excluded").value.split(/[\s,]+/);
  const reply = await ask({
    kind: "setSettings",
    settings: { minSizeBytes: Number.isFinite(mb) && mb > 0 ? Math.round(mb * 1_048_576) : 0, excludedHosts },
  });
  if (reply.kind === "settings") {
    $("savedMore").textContent = t("Saved.");
    showMore(reply.settings);
  }
});

function showMore(settings: Settings): void {
  $<HTMLInputElement>("minMb").value = String(Math.round(settings.minSizeBytes / 1_048_576));
  $<HTMLTextAreaElement>("excluded").value = settings.excludedHosts.join("\n");
}

$<HTMLInputElement>("intercept").addEventListener("change", async (event) => {
  await ask({ kind: "setSettings", settings: { intercept: (event.target as HTMLInputElement).checked } });
});

const size = (n: number | undefined): string => (n ? bytes(n) : "");

async function showExpired(): Promise<void> {
  const reply = await ask({ kind: "expired" });
  if (reply.kind !== "expired") return;
  $("expiredBox").hidden = reply.downloads.length === 0;
  const list = $("expired");
  list.replaceChildren();
  for (const download of reply.downloads) {
    const row = document.createElement("li");
    row.className = "item";
    const label = document.createElement("span");
    label.className = "name";
    label.textContent = download.filename ?? download.url;
    label.title = download.url;
    const button = document.createElement("button");
    button.textContent = t("Refresh link");
    button.addEventListener("click", async () => {
      button.disabled = true;
      button.textContent = t("Looking…");
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
    none.className = "empty";
    none.textContent = t("Nothing found yet. Play the video, then open this again.");
    list.append(none);
  }
  for (const media of reply.media) {
    const row = document.createElement("li");
    row.className = "item";
    const label = document.createElement("span");
    label.className = "name";
    label.textContent = media.name;
    label.title = media.url;
    const info = document.createElement("small");
    info.textContent = [media.kind === "file" ? t("file") : media.kind.toUpperCase(), size(media.size)]
      .filter(Boolean)
      .join(" ");
    const button = document.createElement("button");
    button.textContent = t("Download");
    button.addEventListener("click", async () => {
      button.disabled = true;
      const answer = await ask({ kind: "downloadMedia", media });
      if (answer.kind === "outcome") setResult(describe(answer.outcome));
      button.disabled = false;
      await refresh();
    });
    row.append(label, info, button);
    list.append(row);
  }
}

function bar(percent: number | null, kind: string): HTMLElement {
  const outer = document.createElement("div");
  outer.className = `bar ${kind}`;
  outer.setAttribute("role", "progressbar");
  outer.setAttribute("aria-valuemin", "0");
  outer.setAttribute("aria-valuemax", "100");
  outer.setAttribute("aria-valuenow", String(Math.round(percent ?? 0)));
  const inner = document.createElement("span");
  inner.style.width = `${percent ?? 0}%`;
  outer.append(inner);
  return outer;
}

const LABEL: Record<string, string> = {
  queued: "Waiting",
  probing: "Connecting",
  downloading: "Downloading",
  paused: "Paused",
  retrying: "Retrying",
};
const word = (status: string): string => t(LABEL[status] ?? status);

let seen = new Map<number, number>();
let seenAt = 0;

function showDownloads(rows: DownloadRow[]): void {
  const now = Date.now();
  const rate = speeds(seen, rows, (now - seenAt) / 1000);
  seen = new Map(rows.map((r) => [r.id, r.downloaded]));
  seenAt = now;

  const total = summarize(rows);
  $("nActive").textContent = String(total.running);
  $("nWaiting").textContent = String(total.waiting);
  $("nDone").textContent = String(total.finished);
  $("nFailed").textContent = String(total.failed);
  const sum = [...rate.values()].reduce((a, b) => a + b, 0);
  $("speed").hidden = !(total.running > 0 && sum > 0);
  $("speedNow").textContent = speed(sum);

  const open = rows.filter((r) => ["downloading", "probing", "retrying", "queued", "paused"].includes(r.status));
  open.sort((a, b) => Number(isRunning(b)) - Number(isRunning(a)) || b.id - a.id);
  $("activeBox").hidden = open.length === 0;
  const active = $("active");
  active.replaceChildren();
  for (const row of open.slice(0, 6)) {
    const percent = row.size ? Math.min(100, (row.downloaded / row.size) * 100) : null;
    const li = document.createElement("li");
    li.className = "download";
    const top = document.createElement("div");
    top.className = "row";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = nameOf(row);
    name.title = row.url;
    const state = document.createElement("small");
    state.textContent = percent === null ? word(row.status) : `${Math.floor(percent)}%`;
    top.append(name, state);
    const perSecond = rate.get(row.id) ?? 0;
    const left = row.size && perSecond > 0 ? eta((row.size - row.downloaded) / perSecond) : "";
    const info = document.createElement("small");
    info.textContent = [
      word(row.status),
      row.size ? t("{done} of {total}", { done: bytes(row.downloaded), total: bytes(row.size) }) : bytes(row.downloaded),
      perSecond > 0 ? speed(perSecond) : "",
      left ? t("{time} left", { time: left }) : "",
    ]
      .filter(Boolean)
      .join(" · ");
    li.append(top, bar(percent, isRunning(row) ? "" : "idle"), info);
    active.append(li);
  }
  if (open.length > 6) {
    const more = document.createElement("li");
    more.className = "empty";
    more.textContent = t("and {n} more in the Mohona app", { n: open.length - 6 });
    active.append(more);
  }

  const done = rows.filter((r) => r.status === "completed").sort((a, b) => b.id - a.id).slice(0, 3);
  $("recentBox").hidden = done.length === 0;
  const recent = $("recent");
  recent.replaceChildren();
  for (const row of done) {
    const li = document.createElement("li");
    li.className = "item";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = nameOf(row);
    name.title = row.path ?? row.url;
    const info = document.createElement("small");
    info.textContent = bytes(row.size ?? row.downloaded);
    const open = document.createElement("button");
    open.textContent = t("Open");
    open.addEventListener("click", () => void openFile(row.id, false));
    const show = document.createElement("button");
    show.textContent = t("Show in folder");
    show.addEventListener("click", () => void openFile(row.id, true));
    li.append(name, info, open, show);
    recent.append(li);
  }
}

/** Opens a finished file, or shows it in its folder; a refusal is shown where results are. */
async function openFile(id: number, reveal: boolean): Promise<void> {
  const reply = await ask({ kind: "openFile", id, reveal });
  if (reply.kind === "opened" && reply.error) setResult(reply.error);
}

async function pollDownloads(): Promise<void> {
  const reply = await ask({ kind: "downloads" });
  if (reply.kind === "downloads") showDownloads(reply.downloads);
}

function storageOrNull(): Storage | null {
  try {
    return localStorage;
  } catch {
    return null;
  }
}

/** Puts the chosen language's words on everything the page has already drawn. */
function translatePage(language: Language): void {
  t = translator(language.dict);
  document.documentElement.lang = language.code;
  document.documentElement.dir = language.dir;
  for (const el of document.querySelectorAll<HTMLElement>("[data-t]")) el.textContent = t(el.dataset.t ?? "");
  for (const el of document.querySelectorAll<HTMLElement>("[data-t-label]")) {
    const text = t(el.dataset.tLabel ?? "");
    el.setAttribute("aria-label", text);
    el.title = text;
  }
  for (const el of document.querySelectorAll<HTMLInputElement>("[data-t-placeholder]")) el.placeholder = t(el.dataset.tPlaceholder ?? "");
  const select = $<HTMLSelectElement>("theme");
  const current = select.value;
  select.replaceChildren();
  for (const theme of ALL_THEMES) {
    const option = document.createElement("option");
    option.value = theme.id;
    option.textContent = t(theme.label);
    select.append(option);
  }
  select.value = current || "system";
}

/** The files the page links to, to choose from. */
let pageFiles: PageFile[] = [];
const picked = new Set<string>();
let pageUrl = "";

function drawFiles(): void {
  const list = $("fileList");
  const group = ($("fileGroup") as HTMLSelectElement).value;
  list.replaceChildren();
  const shown = pageFiles.filter((f) => group === "" || f.group === group);
  if (shown.length === 0) {
    const none = document.createElement("li");
    none.className = "empty";
    none.textContent = t("No files found on this page.");
    list.append(none);
  }
  for (const file of shown) {
    const row = document.createElement("li");
    row.className = "item";
    const label = document.createElement("label");
    label.className = "name";
    label.title = file.url;
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = picked.has(file.url);
    box.addEventListener("change", () => {
      if (box.checked) picked.add(file.url);
      else picked.delete(file.url);
      ($("fileGo") as HTMLButtonElement).disabled = picked.size === 0;
    });
    label.append(box, ` ${file.name}`);
    row.append(label);
    list.append(row);
  }
  ($("fileGo") as HTMLButtonElement).disabled = picked.size === 0;
}

async function showPageFiles(): Promise<void> {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  pageFiles = [];
  picked.clear();
  pageUrl = tab?.url ?? "";
  const group = $("fileGroup") as HTMLSelectElement;
  if (group.options.length === 0) {
    group.append(new Option(t("All files"), ""));
    for (const [id, name] of GROUPS) group.append(new Option(t(name), id));
    group.addEventListener("change", drawFiles);
  }
  if (tab?.id !== undefined && /^https?:/i.test(pageUrl)) {
    try {
      const [result] = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: collectLinks });
      pageFiles = filesOf((result?.result as RawLink[] | undefined) ?? [], pageUrl);
    } catch {
      pageFiles = []; // a page the browser does not let extensions read
    }
  }
  drawFiles();
}

async function redraw(): Promise<void> {
  await refresh();
  await showExpired();
  await showMedia();
  await pollDownloads();
}

$("pagefiles").addEventListener("toggle", () => {
  if (($("pagefiles") as HTMLDetailsElement).open) void showPageFiles();
});
const chooseFiles = (on: boolean): void => {
  const group = ($("fileGroup") as HTMLSelectElement).value;
  for (const f of pageFiles) {
    if (group !== "" && f.group !== group) continue;
    if (on) picked.add(f.url);
    else picked.delete(f.url);
  }
  drawFiles();
};
$("fileAll").addEventListener("click", () => chooseFiles(true));
$("fileNone").addEventListener("click", () => chooseFiles(false));
$("fileGo").addEventListener("click", async () => {
  const go = $("fileGo") as HTMLButtonElement;
  go.disabled = true;
  let sent = 0;
  for (const file of pageFiles.filter((f) => picked.has(f.url))) {
    const answer = await ask({ kind: "downloadMedia", media: { url: file.url, kind: "file", name: file.name, referrer: pageUrl } });
    if (answer.kind === "outcome" && answer.outcome?.kind === "error") {
      setResult(describe(answer.outcome));
      break;
    }
    sent += 1;
  }
  if (sent > 0) setResult(`${t("Sent to Mohona")}: ${sent}`);
  picked.clear();
  drawFiles();
  await refresh();
});

function setUpTheme(): void {
  const select = $<HTMLSelectElement>("theme");
  const storage = storageOrNull();
  const current = loadTheme(storage);
  select.value = current;
  applyTheme(document.documentElement, current);
  void chrome.storage.local.set({ "mohona.theme": current }).catch(() => undefined);
  select.addEventListener("change", () => {
    applyTheme(document.documentElement, select.value);
    saveTheme(storage, select.value);
    void chrome.storage.local.set({ "mohona.theme": select.value }).catch(() => undefined);
  });
}

function setUpLanguage(): void {
  const select = $<HTMLSelectElement>("language");
  for (const l of LANGUAGES) {
    const option = document.createElement("option");
    option.value = l.code;
    option.textContent = languageLabel(l);
    option.lang = l.code;
    select.append(option);
  }
  const storage = storageOrNull();
  const current = loadLanguage(storage, navigator.languages ?? [navigator.language]);
  select.value = current.code;
  void chrome.storage.local.set({ "mohona.language": current.code }).catch(() => undefined);
  translatePage(current);
  select.addEventListener("change", () => {
    const next = LANGUAGES.find((l) => l.code === select.value) ?? LANGUAGES[0]!;
    saveLanguage(storage, next.code);
    void chrome.storage.local.set({ "mohona.language": next.code }).catch(() => undefined);
    translatePage(next);
    void redraw();
  });
}

setUpTheme();
setUpLanguage();

void (async () => {
  const reply = await ask({ kind: "getSettings" });
  if (reply.kind === "settings") {
    $<HTMLInputElement>("intercept").checked = reply.settings.intercept;
    $<HTMLInputElement>("autoRefresh").checked = reply.settings.autoRefresh;
    $<HTMLInputElement>("videoButton").checked = reply.settings.videoButton;
    showMore(reply.settings);
  }
  await refresh();
  await showExpired();
  await showMedia();
  await pollDownloads();
  setInterval(() => void pollDownloads().then(refresh), 1500);
})();
