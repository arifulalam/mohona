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

/** The round button before it opens out (its label shows on hover). */
export const BUTTON: Box = { width: 36, height: 32 };

/** Sound players are low and wide. */
export const AUDIO_MIN: Box = { width: 160, height: 28 };

/**
 * Where the button goes: the top right corner of the player, kept inside the
 * window. `least` is how much of the player must be in view.
 */
export function place(
  rect: { left: number; top: number; right: number; bottom: number },
  view: { width: number; height: number },
  button: Box = BUTTON,
  least: Box = { width: MIN_WIDTH / 2, height: MIN_HEIGHT / 2 },
): { left: number; top: number } | null {
  const left = Math.min(rect.right, view.width) - button.width - 8;
  const top = Math.max(rect.top, 0) + 8;
  const visibleHeight = Math.min(rect.bottom, view.height) - Math.max(rect.top, 0);
  const visibleWidth = Math.min(rect.right, view.width) - Math.max(rect.left, 0);
  if (visibleHeight < least.height || visibleWidth < least.width) return null; // mostly off screen
  if (left < rect.left) return null;
  return { left, top };
}

/**
 * Where a sound player's button goes: a low bar has its own controls in the
 * corner, so the button sits beside it (vertically centred), or just above it
 * when there is no room at the side, or else inside its corner.
 */
export function placeBeside(
  rect: { left: number; top: number; right: number; bottom: number },
  view: { width: number; height: number },
  button: Box = BUTTON,
): { left: number; top: number } | null {
  const visibleWidth = Math.min(rect.right, view.width) - Math.max(rect.left, 0);
  const visibleHeight = Math.min(rect.bottom, view.height) - Math.max(rect.top, 0);
  if (visibleWidth < AUDIO_MIN.width / 2 || visibleHeight < AUDIO_MIN.height / 2) return null;
  const centred = rect.top + (rect.bottom - rect.top - button.height) / 2;
  if (rect.right + 6 + button.width <= view.width && centred >= 0 && centred + button.height <= view.height) {
    return { left: rect.right + 6, top: centred };
  }
  if (rect.top - button.height - 4 >= 0) return { left: Math.min(rect.right, view.width) - button.width - 8, top: rect.top - button.height - 4 };
  return place(rect, view, button, { width: 80, height: 20 });
}

/** The players on a page that deserve a button. */
export function videosToMark(root: ParentNode): HTMLVideoElement[] {
  return Array.from(root.querySelectorAll("video")).filter((video) => {
    const box = video.getBoundingClientRect();
    return bigEnough({ width: box.width, height: box.height });
  });
}

/** Sound players that show their own controls (hidden ones are page machinery). */
export function soundsToMark(root: ParentNode): HTMLAudioElement[] {
  return Array.from(root.querySelectorAll("audio")).filter((audio) => {
    if (!audio.controls) return false;
    const box = audio.getBoundingClientRect();
    return box.width >= AUDIO_MIN.width && box.height >= AUDIO_MIN.height;
  });
}

/** Everything that gets a button: players and sounds. */
export function playersToMark(root: ParentNode): HTMLMediaElement[] {
  return [...videosToMark(root), ...soundsToMark(root)];
}

/** A direct address of the video, when it has one (not `blob:`, which is no address to download). */
export function directSource(video: HTMLMediaElement): string | null {
  const src = video.currentSrc || video.getAttribute("src") || "";
  return /^https?:\/\//i.test(src) ? src : null;
}
