// Runs in pages. Finds videos and puts one small button on each; everything
// else (what can be downloaded, the download itself) is the background script's job.

import type { Reply, Request } from "./messages";
import type { Media } from "./sniffer";
import { allowedOn, directSource, place, videosToMark, withHidden } from "./overlay";
import { DEFAULTS, loadSettings, sanitize, type Settings } from "./settings";

let settings: Settings = DEFAULTS;
const buttons = new Map<HTMLVideoElement, Button>();

function ask(request: Request): Promise<Reply | undefined> {
  return chrome.runtime.sendMessage(request).catch(() => undefined) as Promise<Reply | undefined>;
}

const STYLE = `
  :host { all: initial; }
  .wrap { position: fixed; z-index: 2147483647; font: 13px system-ui, sans-serif; }
  button { all: unset; cursor: pointer; box-sizing: border-box; }
  .main { background: rgba(20,20,20,.72); color: #fff; padding: 5px 10px; border-radius: 14px; opacity: .55; transition: opacity .15s; white-space: nowrap; }
  .wrap:hover .main, .main:focus-visible { opacity: 1; }
  .panel { position: absolute; right: 0; top: 34px; width: 280px; max-height: 260px; overflow: auto; background: #fff; color: #111; border-radius: 8px; padding: 8px; box-shadow: 0 4px 18px rgba(0,0,0,.35); }
  .panel p { margin: 4px 0; }
  .item { display: flex; gap: 6px; align-items: center; margin: 4px 0; }
  .item span { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item small { color: #666; }
  .item button, .links button { background: #1565c0; color: #fff; padding: 3px 8px; border-radius: 4px; }
  .links { margin-top: 6px; display: flex; gap: 10px; }
  .links button { background: none; color: #1565c0; padding: 0; text-decoration: underline; }
`;

function sizeText(bytes: number | undefined): string {
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

class Button {
  readonly host = document.createElement("mohona-video-button");
  private readonly wrap: HTMLDivElement;
  private readonly panel: HTMLDivElement;

  constructor(private readonly video: HTMLVideoElement) {
    const root = this.host.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = STYLE;
    this.wrap = document.createElement("div");
    this.wrap.className = "wrap";
    const main = document.createElement("button");
    main.className = "main";
    main.textContent = "⬇ Mohona";
    main.title = "Download this video with Mohona";
    this.panel = document.createElement("div");
    this.panel.className = "panel";
    this.panel.hidden = true;
    this.wrap.append(main, this.panel);
    root.append(style, this.wrap);
    main.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void this.toggle();
    });
    // The page must not see (or react to) clicks on the button.
    for (const type of ["mousedown", "mouseup", "pointerdown", "pointerup", "dblclick"]) {
      this.wrap.addEventListener(type, (e) => e.stopPropagation());
    }
    document.documentElement.append(this.host);
  }

  position(): void {
    const rect = this.video.getBoundingClientRect();
    const spot = place(rect, { width: window.innerWidth, height: window.innerHeight });
    this.wrap.style.display = spot ? "" : "none";
    if (spot) {
      this.wrap.style.left = `${spot.left}px`;
      this.wrap.style.top = `${spot.top}px`;
    }
    if (!spot) this.panel.hidden = true;
  }

  remove(): void {
    this.host.remove();
  }

  private text(message: string): void {
    const p = document.createElement("p");
    p.textContent = message;
    this.panel.replaceChildren(p);
  }

  private async toggle(): Promise<void> {
    if (!this.panel.hidden) {
      this.panel.hidden = true;
      return;
    }
    this.panel.hidden = false;
    this.text("Looking for downloads…");
    const direct = directSource(this.video);
    const reply = await ask({ kind: "pageMedia", direct: direct ?? undefined, page: location.href });
    if (!reply || reply.kind !== "media") {
      this.text("Mohona is not available. Is it installed and running?");
      return;
    }
    this.show(reply.media);
  }

  private show(media: Media[]): void {
    const parts: HTMLElement[] = [];
    if (media.length === 0) {
      const p = document.createElement("p");
      p.textContent = /(^|\.)(youtube\.com|youtu\.be)$/.test(location.hostname)
        ? "YouTube sends video and sound in many small signed pieces, so there is no single file to take yet. Support for it is planned."
        : "Nothing to download was found yet. Press play, wait a moment, and try again.";
      parts.push(p);
    }
    for (const item of media) {
      const row = document.createElement("div");
      row.className = "item";
      const label = document.createElement("span");
      label.textContent = item.name;
      label.title = item.url;
      const info = document.createElement("small");
      info.textContent = [item.kind === "file" ? "file" : item.kind.toUpperCase(), sizeText(item.size)]
        .filter(Boolean)
        .join(" ");
      const go = document.createElement("button");
      go.textContent = "Download";
      go.addEventListener("click", async () => {
        go.disabled = true;
        const reply = await ask({ kind: "downloadMedia", media: item });
        const outcome = reply?.kind === "outcome" ? reply.outcome : null;
        go.textContent = outcome && outcome.kind !== "error" && outcome.kind !== "duplicate" ? "Sent ✓" : "Failed";
        if (outcome && (outcome.kind === "error" || outcome.kind === "duplicate")) go.title = outcome.message;
      });
      row.append(label, info, go);
      parts.push(row);
    }
    const links = document.createElement("div");
    links.className = "links";
    const hide = document.createElement("button");
    hide.textContent = `Hide on ${location.hostname}`;
    hide.addEventListener("click", async () => {
      await ask({ kind: "setSettings", settings: { buttonHiddenHosts: withHidden(settings.buttonHiddenHosts, location.hostname) } });
    });
    const close = document.createElement("button");
    close.textContent = "Close";
    close.addEventListener("click", () => (this.panel.hidden = true));
    links.append(hide, close);
    parts.push(links);
    this.panel.replaceChildren(...parts);
  }
}

function scan(): void {
  const show = allowedOn(settings, location.hostname);
  const wanted = show ? new Set(videosToMark(document)) : new Set<HTMLVideoElement>();
  for (const [video, button] of buttons) {
    if (!wanted.has(video) || !video.isConnected) {
      button.remove();
      buttons.delete(video);
    }
  }
  for (const video of wanted) {
    if (!buttons.has(video)) buttons.set(video, new Button(video));
  }
  for (const button of buttons.values()) button.position();
}

let timer: ReturnType<typeof setTimeout> | undefined;
function later(): void {
  clearTimeout(timer);
  timer = setTimeout(scan, 300);
}

async function start(): Promise<void> {
  settings = await loadSettings();
  chrome.storage.onChanged.addListener((_changes, area) => {
    if (area !== "sync") return;
    void loadSettings().then((s) => {
      settings = sanitize(s);
      scan();
    });
  });
  new MutationObserver(later).observe(document.documentElement, { childList: true, subtree: true });
  for (const type of ["scroll", "resize"]) window.addEventListener(type, scan, { capture: true, passive: true });
  document.addEventListener("play", later, true);
  document.addEventListener("loadedmetadata", later, true);
  // Players that resize themselves without changing the page's structure.
  setInterval(scan, 1500);
  scan();
}

void start();
