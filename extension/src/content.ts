// Runs in pages. Finds videos and puts one small button on each; everything
// else (what can be downloaded, the download itself) is the background script's job.

import type { Reply, Request } from "./messages";
import type { Media } from "./sniffer";
import type { Colors, Look } from "./look";
import type { VideoChoice } from "./videochoice";
import { allowedOn, BUTTON, directSource, place, placeBeside, playersToMark, withHidden } from "./overlay";
import { duplicateNote, savedChoice, videoPicker } from "./widgets";
import { DEFAULTS, loadSettings, sanitize, type Settings } from "./settings";

let settings: Settings = DEFAULTS;
let look: Look | null = null;
const buttons = new Map<HTMLMediaElement, Button>();

function ask(request: Request): Promise<Reply | undefined> {
  return chrome.runtime.sendMessage(request).catch(() => undefined) as Promise<Reply | undefined>;
}

/** A text in the person's language (English until the background has answered). */
function t(text: string, vars?: Record<string, string>): string {
  const words = look?.words[text] ?? text;
  return vars ? words.replace(/\{(\w+)\}/g, (whole, name: string) => vars[name] ?? whole) : words;
}

const STYLE = `
  :host { all: initial; }
  .wrap {
    position: fixed; z-index: 2147483647; font: 13px/1.35 system-ui, "Segoe UI", sans-serif; transform: translateX(-100%);
    --panel: #ffffff; --text: #1b1f27; --muted: #5a6370; --line: #e2e5ea; --accent: #1259b0; --accent-text: #ffffff;
    --ok: #1b7f3b; --bad: #b3261e; --hero: linear-gradient(135deg,#2b78d4,#1259b0); --hero-text: #ffffff;
    color: var(--text);
  }
  @media (prefers-color-scheme: dark) {
    .wrap.auto {
      --panel: #1c2027; --text: #e8eaee; --muted: #9aa3b0; --line: #2d333d; --accent: #5b9df0; --accent-text: #08121f;
      --ok: #5cc27a; --bad: #ff8a80; --hero: linear-gradient(135deg,#2a5fa8,#14325c);
    }
  }
  button { all: unset; cursor: pointer; box-sizing: border-box; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .fab {
    display: flex; align-items: center; height: 32px; min-width: 32px; padding: 0 8px; border-radius: 16px;
    background: var(--hero); color: var(--hero-text); opacity: .72;
    box-shadow: 0 2px 10px rgba(0,0,0,.38), inset 0 0 0 1px rgba(255,255,255,.22);
    transition: opacity .15s, transform .15s, box-shadow .15s;
  }
  .fab svg { width: 16px; height: 16px; flex: none; margin: 0 auto; }
  .fab .label { max-width: 0; overflow: hidden; white-space: nowrap; font-weight: 600; letter-spacing: .2px; transition: max-width .22s ease, margin .22s ease; }
  .wrap:hover .fab, .wrap.open .fab, .fab:focus-visible { opacity: 1; box-shadow: 0 4px 16px rgba(0,0,0,.45), inset 0 0 0 1px rgba(255,255,255,.3); }
  .wrap:hover .label, .wrap.open .label, .fab:focus-visible .label { max-width: 160px; margin-inline: 8px 2px; }
  .fab:active { transform: scale(.96); }
  .panel {
    position: absolute; right: 0; top: 40px; width: 300px; max-height: 300px; overflow: auto; border-radius: 14px;
    background: var(--panel); color: var(--text); border: 1px solid var(--line);
    box-shadow: 0 10px 34px rgba(0,0,0,.4); transform-origin: top right; animation: pop .14s ease-out;
  }
  @keyframes pop { from { opacity: 0; transform: scale(.96) translateY(-4px); } to { opacity: 1; transform: none; } }
  .head { display: flex; align-items: center; gap: 8px; padding: 9px 10px; background: var(--hero); color: var(--hero-text); border-radius: 13px 13px 0 0; font-weight: 700; }
  .head .title { flex: 1; }
  .head svg { width: 15px; height: 15px; }
  .x { width: 22px; height: 22px; display: grid; place-items: center; border-radius: 11px; font-size: 16px; line-height: 1; background: rgba(255,255,255,.18); }
  .x:hover { background: rgba(255,255,255,.32); }
  .body { padding: 6px 10px 8px; }
  .body p { margin: 8px 0; color: var(--muted); }
  .item { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; padding: 7px 0; border-top: 1px solid var(--line); }
  .pick { flex-basis: 100%; display: flex; gap: 6px; }
  select { flex: 1; min-width: 0; font: inherit; color: var(--text); background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 3px 6px; }
  .note { flex-basis: 100%; }
  .note p { margin: 2px 0 6px; color: var(--text); }
  .note .actions { display: flex; gap: 6px; flex-wrap: wrap; }
  .note .actions button:not(.go) { padding: 5px 11px; border-radius: 14px; border: 1px solid var(--line); font-weight: 600; }
  .note .problem { display: block; margin-top: 4px; color: var(--bad); }
  .item:first-child { border-top: 0; }
  .meta { flex: 1; min-width: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .sub { display: flex; gap: 6px; align-items: center; color: var(--muted); font-size: 11px; margin-top: 2px; }
  .chip { padding: 0 6px; border-radius: 8px; background: var(--line); color: var(--text); font-weight: 700; font-size: 10px; letter-spacing: .4px; }
  .go { padding: 5px 11px; border-radius: 14px; background: var(--accent); color: var(--accent-text); font-weight: 700; white-space: nowrap; }
  .go:hover { filter: brightness(1.1); }
  .go[disabled] { opacity: .75; cursor: default; }
  .go.ok { background: var(--ok); color: #fff; }
  .go.bad { background: var(--bad); color: #fff; }
  .foot { display: flex; justify-content: space-between; padding: 7px 0 0; border-top: 1px solid var(--line); margin-top: 2px; }
  .foot button { color: var(--muted); font-size: 12px; text-decoration: underline; }
  .foot button:hover { color: var(--text); }
`;

const ICON =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 4v11"/><path d="M7 11l5 5 5-5"/><path d="M5 20h14"/></svg>';

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

/** The look of the person's theme as CSS variables on the button's box. */
function paint(wrap: HTMLElement, colors: Colors | null): void {
  wrap.classList.toggle("auto", colors === null);
  const vars: Record<string, string | undefined> = colors
    ? {
        "--panel": colors.panel, "--text": colors.text, "--muted": colors.muted, "--line": colors.line, "--accent": colors.accent,
        "--accent-text": colors.accentText, "--ok": colors.ok, "--bad": colors.bad, "--hero": colors.hero, "--hero-text": colors.heroText,
      }
    : {};
  for (const name of ["--panel", "--text", "--muted", "--line", "--accent", "--accent-text", "--ok", "--bad", "--hero", "--hero-text"]) {
    const value = vars[name];
    if (value) wrap.style.setProperty(name, value);
    else wrap.style.removeProperty(name);
  }
}

class Button {
  readonly host = document.createElement("mohona-video-button");
  private readonly wrap: HTMLDivElement;
  private readonly panel: HTMLDivElement;
  private readonly main: HTMLButtonElement;
  private readonly label: HTMLSpanElement;
  private readonly sound: boolean;

  constructor(private readonly media: HTMLMediaElement) {
    this.sound = media instanceof HTMLAudioElement;
    const root = this.host.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = STYLE;
    this.wrap = document.createElement("div");
    this.wrap.className = "wrap";
    this.main = document.createElement("button");
    this.main.className = "fab";
    this.main.insertAdjacentHTML("beforeend", ICON);
    this.label = document.createElement("span");
    this.label.className = "label";
    this.main.append(this.label);
    this.panel = document.createElement("div");
    this.panel.className = "panel";
    this.panel.hidden = true;
    this.wrap.append(this.main, this.panel);
    root.append(style, this.wrap);
    this.restyle();
    this.main.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void this.toggle();
    });
    // The page must not see (or react to) clicks on the button.
    for (const type of ["mousedown", "mouseup", "pointerdown", "pointerup", "dblclick", "click"]) {
      this.wrap.addEventListener(type, (e) => e.stopPropagation());
    }
    document.documentElement.append(this.host);
  }

  /** Words, direction and colours, once the background has told us what the person chose. */
  restyle(): void {
    this.label.textContent = "Mohona";
    this.main.title = t("Download with Mohona");
    this.main.setAttribute("aria-label", t("Download with Mohona"));
    this.wrap.dir = look?.dir ?? "ltr";
    paint(this.wrap, look?.colors ?? null);
  }

  position(): void {
    const rect = this.media.getBoundingClientRect();
    const view = { width: window.innerWidth, height: window.innerHeight };
    const spot = this.sound ? placeBeside(rect, view) : place(rect, view);
    this.wrap.style.display = spot ? "" : "none";
    if (spot) {
      // The box is anchored by its right edge, so it opens out to the left.
      this.wrap.style.left = `${spot.left + BUTTON.width}px`;
      this.wrap.style.top = `${spot.top}px`;
    }
    if (!spot) this.close();
  }

  remove(): void {
    this.host.remove();
  }

  private close(): void {
    this.panel.hidden = true;
    this.wrap.classList.remove("open");
  }

  private text(message: string): void {
    const body = this.frame();
    const p = document.createElement("p");
    p.textContent = message;
    body.append(p);
  }

  /** The card: a coloured header with a close button, and a body to fill. */
  private frame(): HTMLElement {
    const head = document.createElement("div");
    head.className = "head";
    head.insertAdjacentHTML("beforeend", ICON);
    const title = document.createElement("span");
    title.className = "title";
    title.textContent = "Mohona";
    const x = document.createElement("button");
    x.className = "x";
    x.textContent = "×";
    x.title = t("Close");
    x.setAttribute("aria-label", t("Close"));
    x.addEventListener("click", () => this.close());
    head.append(title, x);
    const body = document.createElement("div");
    body.className = "body";
    this.panel.replaceChildren(head, body);
    return body;
  }

  private async toggle(): Promise<void> {
    if (!this.panel.hidden) {
      this.close();
      return;
    }
    this.panel.hidden = false;
    this.wrap.classList.add("open");
    this.text(t("Looking for downloads…"));
    const direct = directSource(this.media);
    const reply = await ask({ kind: "pageMedia", direct: direct ?? undefined, page: location.href });
    if (!reply || reply.kind !== "media") {
      this.text(t("Mohona is not available. Is it installed and running?"));
      return;
    }
    this.show(reply.media, await savedChoice());
  }

  private show(media: Media[], start: VideoChoice): void {
    const body = this.frame();
    if (media.length === 0) {
      const p = document.createElement("p");
      p.textContent = t("Nothing to download was found yet. Press play, wait a moment, and try again.");
      body.append(p);
    }
    for (const item of media) {
      const row = document.createElement("div");
      row.className = "item";
      const meta = document.createElement("div");
      meta.className = "meta";
      const name = document.createElement("div");
      name.className = "name";
      name.textContent = item.name;
      name.title = item.url;
      const sub = document.createElement("div");
      sub.className = "sub";
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.textContent = item.kind === "file" ? t("file").toUpperCase() : item.kind.toUpperCase();
      const size = document.createElement("span");
      size.textContent = sizeText(item.size);
      sub.append(chip, size);
      meta.append(name, sub);
      const go = document.createElement("button");
      go.className = "go";
      go.textContent = t("Download");
      // A video page: how sharp, and in which format (or sound only).
      const picker = item.video ? videoPicker(t, start) : null;
      const send = async (again: boolean) => {
        const media = picker ? { ...item, choice: picker.value() } : item;
        const reply = await ask({ kind: "downloadMedia", media, ...(again ? { again: true } : {}) });
        return reply?.kind === "outcome" ? reply.outcome : null;
      };
      const finish = (outcome: Awaited<ReturnType<typeof send>>): void => {
        if (outcome?.kind === "duplicate") {
          // Downloaded before: open it, or say the file is gone and offer to fetch it again.
          const id = outcome.id;
          const opened = async (reveal: boolean): Promise<string | null> => {
            if (id === undefined) return null;
            const answer = await ask({ kind: "openFile", id, reveal });
            return answer?.kind === "opened" ? answer.error : null;
          };
          row.querySelector(".note")?.remove();
          row.append(
            duplicateNote(t, outcome, {
              open: () => opened(false),
              reveal: () => opened(true),
              again: async () => {
                const next = await send(true);
                row.querySelector(".note")?.remove();
                go.hidden = false;
                finish(next);
              },
            }),
          );
          go.hidden = true;
          return;
        }
        const sent = outcome !== null && outcome.kind !== "error";
        go.hidden = false;
        go.disabled = sent;
        go.textContent = sent ? `${t("Sent")} ✓` : t("Failed");
        go.classList.remove("ok", "bad");
        go.classList.add(sent ? "ok" : "bad");
        if (outcome?.kind === "error") go.title = outcome.message;
      };
      go.addEventListener("click", async () => {
        go.disabled = true;
        go.textContent = t("Sending…");
        finish(await send(false));
        if (!go.hidden && !go.classList.contains("ok")) go.disabled = false;
      });
      row.append(meta, go);
      if (picker) row.append(picker.element);
      body.append(row);
    }
    const foot = document.createElement("div");
    foot.className = "foot";
    const hide = document.createElement("button");
    hide.textContent = t("Hide on {site}", { site: location.hostname });
    hide.addEventListener("click", async () => {
      await ask({ kind: "setSettings", settings: { buttonHiddenHosts: withHidden(settings.buttonHiddenHosts, location.hostname) } });
    });
    const close = document.createElement("button");
    close.textContent = t("Close");
    close.addEventListener("click", () => this.close());
    foot.append(hide, close);
    body.append(foot);
  }
}

function scan(): void {
  const show = allowedOn(settings, location.hostname);
  const wanted = show ? new Set(playersToMark(document)) : new Set<HTMLMediaElement>();
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

async function loadLook(): Promise<void> {
  const reply = await ask({ kind: "look" });
  if (reply?.kind === "look") {
    look = reply.look;
    for (const button of buttons.values()) button.restyle();
  }
}

async function start(): Promise<void> {
  settings = await loadSettings();
  void loadLook();
  // A theme or language chosen in the popup shows on the next page, and on this one at once.
  chrome.storage.onChanged.addListener((_changes, area) => {
    if (area === "local") void loadLook();
  });
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
