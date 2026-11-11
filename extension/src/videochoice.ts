// What the person can ask of a video page: how sharp, and which file format. The values are the
// ones the engine knows (crates/media/src/ytdlp.rs: QUALITIES, CONTAINERS, AUDIO).

export const QUALITIES = ["best", "2160", "1440", "1080", "720", "480", "360", "audio"] as const;
export const CONTAINERS = ["mp4", "mkv", "webm", "mov", "avi"] as const;
export const AUDIO = ["mp3", "m4a", "opus", "ogg", "flac", "wav", "aac"] as const;

export interface VideoChoice {
  quality: string;
  container: string;
  audio: string;
}

export const DEFAULT_CHOICE: VideoChoice = { quality: "best", container: "mp4", audio: "mp3" };

const pick = (value: unknown, allowed: readonly string[], fallback: string): string =>
  typeof value === "string" && allowed.includes(value) ? value : fallback;

/** A remembered or half-filled choice made safe: anything unknown becomes the default. */
export function cleanChoice(value: Partial<VideoChoice> | null | undefined): VideoChoice {
  return {
    quality: pick(value?.quality, QUALITIES, DEFAULT_CHOICE.quality),
    container: pick(value?.container, CONTAINERS, DEFAULT_CHOICE.container),
    audio: pick(value?.audio, AUDIO, DEFAULT_CHOICE.audio),
  };
}

/** The formats to offer for a quality: sound formats for "audio", video formats otherwise. */
export const formatsFor = (quality: string): readonly string[] => (quality === "audio" ? AUDIO : CONTAINERS);
