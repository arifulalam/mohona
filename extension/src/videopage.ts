import type { Media } from "./sniffer";

/**
 * Pages of video sites whose real file is hidden behind a program that reads the page (yt-dlp, which
 * the engine runs). Mirrors `mohona_media::ytdlp::is_site_page`: a single video, never a channel or search.
 */
const SITES = ["youtube.com", "youtu.be", "youtube-nocookie.com", "vimeo.com", "dailymotion.com"];

export function isVideoPage(address: string | undefined | null): boolean {
  if (!address) return false;
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    return false;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return false;
  const host = url.hostname.toLowerCase();
  if (!SITES.some((s) => host === s || host.endsWith(`.${s}`))) return false;
  const path = url.pathname;
  if (host === "youtu.be") return path.length > 1;
  if (host.endsWith("youtube.com") || host.endsWith("youtube-nocookie.com")) {
    return (
      (path === "/watch" && (url.searchParams.get("v") ?? "") !== "") ||
      ["/shorts/", "/live/", "/embed/", "/v/"].some((p) => path.startsWith(p))
    );
  }
  if (host.endsWith("vimeo.com")) {
    const last = path.replace(/^\/+|\/+$/g, "").split("/").pop() ?? "";
    return /^[0-9]+$/.test(last);
  }
  return path.startsWith("/video/");
}

/** The page itself as something to download, when it is a video page. */
export function pageVideo(address: string | undefined | null, title: string | undefined | null): Media | null {
  if (!address || !isVideoPage(address)) return null;
  const name = (title ?? "").replace(/\s*[-–|]\s*(YouTube|Vimeo|Dailymotion)\s*$/i, "").replace(/^\(\d+\)\s*/, "").trim();
  return { url: address, kind: "file", name: name || "This video", referrer: address, video: true };
}

/** File endings that say an address is a file, not a web page. */
const PAGE_ENDINGS = ["html", "htm", "shtml", "php", "asp", "aspx", "jsp"];

/**
 * Any other web page, offered as "the video on this page, if there is one": the engine asks yt-dlp,
 * which knows Facebook, X, Instagram, Pinterest, SoundCloud and a thousand more. Mirrors
 * `mohona_media::ytdlp::could_be_page`.
 */
export function pageGuess(address: string | undefined | null): Media | null {
  if (!address || isVideoPage(address)) return null;
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  const host = url.hostname.toLowerCase();
  if (SITES.some((s) => host === s || host.endsWith(`.${s}`))) return null; // a channel, a search...
  const last = url.pathname.split("/").pop() ?? "";
  const dot = last.lastIndexOf(".");
  if (dot >= 0 && !PAGE_ENDINGS.includes(last.slice(dot + 1).toLowerCase())) return null;
  return { url: address, kind: "file", name: "Video on this page", referrer: address, video: true };
}
