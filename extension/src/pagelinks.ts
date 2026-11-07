/**
 * The files a web page offers: pictures, videos, documents, archives and the like, found in the
 * page's links. The same rules as the app's site grabber (`mohona-grabber`).
 */

export interface RawLink {
  url: string;
  /** `a`: a link, `download`: a link marked download, `media`: something shown in the page. */
  kind: "a" | "download" | "media";
}

export interface PageFile {
  url: string;
  name: string;
  group: string;
}

const IMAGES = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "tif", "tiff", "ico", "avif", "heic"];
const VIDEO = ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp"];
const AUDIO = ["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus"];
const DOCUMENTS = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "odt", "ods", "odp", "epub", "csv"];
const ARCHIVES = ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "iso"];
const PROGRAMS = ["exe", "msi", "dmg", "apk", "deb", "rpm", "appimage", "pkg"];
const PAGES = ["html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp", "cgi"];
const JUNK = ["css", "js", "map", "woff", "woff2", "ttf", "eot", "otf"];

/** The groups to choose from, in the order shown: id and English name. */
export const GROUPS: [string, string][] = [
  ["images", "Pictures"],
  ["video", "Videos"],
  ["audio", "Audio"],
  ["documents", "Documents"],
  ["archives", "Archives"],
  ["programs", "Programs"],
  ["other", "Other"],
];

export function groupOf(extension: string): string {
  for (const [name, list] of [
    ["images", IMAGES],
    ["video", VIDEO],
    ["audio", AUDIO],
    ["documents", DOCUMENTS],
    ["archives", ARCHIVES],
    ["programs", PROGRAMS],
  ] as const) {
    if (list.includes(extension)) return name;
  }
  return "other";
}

/** The ending of an address (lower case, 1 to 8 letters or digits), if its last part has one. */
function extensionOf(url: URL): string | null {
  const last = url.pathname.split("/").pop() ?? "";
  const dot = last.lastIndexOf(".");
  if (dot < 0) return null;
  const ext = last.slice(dot + 1);
  return ext.length >= 1 && ext.length <= 8 && /^[A-Za-z0-9]+$/.test(ext) ? ext.toLowerCase() : null;
}

function nameOf(url: URL): string {
  const last = url.pathname.split("/").filter(Boolean).pop() ?? "";
  let name = last;
  try {
    name = decodeURIComponent(last);
  } catch {
    // keep it as it is
  }
  name = name.replace(/[<>:"|?*\\/\u0000-\u001f]/g, "_").trim();
  return name || url.hostname;
}

/** The files among `links` (the page's own address is only used to skip the page itself), once each, in page order. */
export function filesOf(links: RawLink[], pageUrl: string): PageFile[] {
  const seen = new Set<string>([pageUrl.split("#")[0]!]);
  const out: PageFile[] = [];
  for (const link of links) {
    let url: URL;
    try {
      url = new URL(link.url);
    } catch {
      continue;
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") continue;
    url.hash = "";
    const ext = extensionOf(url);
    const pageLike = ext === null || PAGES.includes(ext);
    const file = link.kind === "a" ? !pageLike : true;
    if (!file) continue;
    if (link.kind === "a" && ext !== null && JUNK.includes(ext)) continue;
    if (seen.has(url.href)) continue;
    seen.add(url.href);
    out.push({ url: url.href, name: nameOf(url), group: groupOf(ext ?? "") });
  }
  return out;
}

/**
 * Runs inside the page (it must not use anything from outside): every address the page links to or shows.
 * `chrome.scripting.executeScript` serializes this function, so it stands alone.
 */
export function collectLinks(): RawLink[] {
  const out: RawLink[] = [];
  const add = (value: string | null | undefined, kind: RawLink["kind"]): void => {
    if (value) out.push({ url: value, kind });
  };
  for (const a of Array.from(document.querySelectorAll<HTMLAnchorElement>("a[href], area[href]"))) {
    add(a.href, a.hasAttribute("download") ? "download" : "a");
  }
  for (const el of Array.from(document.querySelectorAll<HTMLElement>("img, video, audio, source, embed, track"))) {
    add((el as HTMLImageElement).currentSrc || (el as HTMLImageElement).src, "media");
    const srcset = el.getAttribute("srcset");
    if (srcset) {
      for (const part of srcset.split(",")) {
        const first = part.trim().split(/\s+/)[0];
        if (first) add(new URL(first, document.baseURI).href, "media");
      }
    }
  }
  return out;
}
