/** Media found in the requests a page makes. */

export type MediaKind = "hls" | "dash" | "file";

export interface Media {
  url: string;
  kind: MediaKind;
  /** A name to save it under, without extension. */
  name: string;
  contentType?: string;
  size?: number;
  /** The page's address (or its origin), for the download's Referer. */
  referrer?: string;
}

/** The longest list kept for one tab. */
export const MAX_PER_TAB = 50;

/** Smaller files are not worth showing (icons, sounds on buttons). */
const MIN_FILE_BYTES = 200_000;

const HLS_TYPES = ["application/vnd.apple.mpegurl", "application/x-mpegurl", "audio/mpegurl", "audio/x-mpegurl"];
const FILE_EXTENSIONS = ["mp4", "webm", "mkv", "mov", "flv", "mp3", "m4a", "ogg", "opus", "wav", "flac", "m4v"];
// Pieces of a stream, not something to download by themselves.
const SEGMENT_TYPES = ["video/mp2t", "video/iso.segment", "audio/aac", "audio/mp2t", "video/vnd.dlna.mpeg-tts"];

// Pieces of a stream by their file name (DASH and CMAF segments, TS chunks).
const SEGMENT_EXTENSIONS = ["ts", "m4s", "m4f", "cmfv", "cmfa", "cmft", "fmp4", "init"];

/** Chunks of an adaptive player (YouTube and alike): signed, ranged, and video and audio apart. */
function looksLikeChunk(parsed: URL): boolean {
  const q = parsed.searchParams;
  return (
    parsed.pathname.endsWith("/videoplayback") ||
    q.has("range") ||
    q.has("bytestart") ||
    q.has("byteend") ||
    /(^|[/_-])(seg|segment|frag|fragment|chunk)[-_]?\d+/i.test(parsed.pathname)
  );
}

function extensionOf(path: string): string {
  const name = path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

/** `bytes 0-99/12345` gives 12345. */
export function totalFromRange(contentRange: string | undefined): number | undefined {
  const total = /\/(\d+)\s*$/.exec(contentRange ?? "")?.[1];
  return total ? Number(total) : undefined;
}

/** What a response is, from its address and headers; null when it is not media worth listing. */
export function classify(
  url: string,
  contentType: string | undefined,
  size: number | undefined,
): MediaKind | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  const type = (contentType ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
  const ext = extensionOf(parsed.pathname);
  if (ext === "m3u8" || HLS_TYPES.includes(type)) return "hls";
  if (ext === "mpd" || type === "application/dash+xml") return "dash";
  if (SEGMENT_TYPES.includes(type) || SEGMENT_EXTENSIONS.includes(ext) || looksLikeChunk(parsed)) return null;
  const mediaType = type.startsWith("video/") || type.startsWith("audio/");
  if (!mediaType && !FILE_EXTENSIONS.includes(ext)) return null;
  if (!mediaType && type && type !== "application/octet-stream") return null;
  if (typeof size === "number" && size < MIN_FILE_BYTES) return null;
  return "file";
}

/** A file name from the address: `/a/b/lesson-1.mp4` gives `lesson-1`; `/x/master.m3u8` gives `x`. */
export function nameFromUrl(url: string): string {
  let parts: string[] = [];
  try {
    parts = new URL(url).pathname.split("/").filter(Boolean).map(decodeURIComponentSafe);
  } catch {
    return "video";
  }
  const generic = new Set(["index", "master", "playlist", "manifest", "main", "stream", "video", "chunklist"]);
  let last = (parts.pop() ?? "").replace(/\.[A-Za-z0-9]{1,5}$/, "");
  if (!last || generic.has(last.toLowerCase())) last = parts.pop() ?? last;
  const clean = last
    // eslint-disable-next-line no-control-regex
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 80);
  return clean || "video";
}

function decodeURIComponentSafe(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

/** Adds (or refreshes) one entry; newest first, no duplicates, at most MAX_PER_TAB. */
export function addMedia(list: Media[], media: Media): Media[] {
  const rest = list.filter((m) => m.url !== media.url);
  return [media, ...rest].slice(0, MAX_PER_TAB);
}
