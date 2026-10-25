// The small download button on videos in pages: what to mark, where to put
// the button, and when to stay away. The page itself is only looked at to find
// <video> elements and their size; nothing is read from it or sent anywhere
// until the person clicks the button.

import { hostMatches, type Settings } from "./settings";

export interface Box {
  width: number;
  height: number;
}

/** Players smaller than this are previews, ads or decoration. */
export const MIN_WIDTH = 240;
export const MIN_HEIGHT = 135;

export function bigEnough(box: Box): boolean {
  return box.width >= MIN_WIDTH && box.height >= MIN_HEIGHT;
}

/** Whether the button may be shown on this site at all. */
export function allowedOn(settings: Pick<Settings, "videoButton" | "buttonHiddenHosts">, host: string): boolean {
  return settings.videoButton && !settings.buttonHiddenHosts.some((site) => hostMatches(host, site));
}

/** The settings change that hides the button on `host` (a host is added once). */
export function withHidden(hidden: string[], host: string): string[] {
  const site = host.toLowerCase().replace(/^www\./, "");
  return hidden.some((h) => h === site) ? hidden : [...hidden, site];
}

/** Where the button goes: the top right corner of the video, kept inside the window. */
export function place(
  rect: { left: number; top: number; right: number; bottom: number },
  view: { width: number; height: number },
  button: Box = { width: 96, height: 28 },
): { left: number; top: number } | null {
  const left = Math.min(rect.right, view.width) - button.width - 8;
  const top = Math.max(rect.top, 0) + 8;
  const visibleHeight = Math.min(rect.bottom, view.height) - Math.max(rect.top, 0);
  const visibleWidth = Math.min(rect.right, view.width) - Math.max(rect.left, 0);
  if (visibleHeight < MIN_HEIGHT / 2 || visibleWidth < MIN_WIDTH / 2) return null; // mostly off screen
  if (left < rect.left) return null;
  return { left, top };
}

/** The players on a page that deserve a button. */
export function videosToMark(root: ParentNode): HTMLVideoElement[] {
  return Array.from(root.querySelectorAll("video")).filter((video) => {
    const box = video.getBoundingClientRect();
    return bigEnough({ width: box.width, height: box.height });
  });
}

/** A direct address of the video, when it has one (not `blob:`, which is no address to download). */
export function directSource(video: HTMLVideoElement): string | null {
  const src = video.currentSrc || video.getAttribute("src") || "";
  return /^https?:\/\//i.test(src) ? src : null;
}
