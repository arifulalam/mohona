import { describe, expect, it } from "vitest";
import { addMedia, classify, MAX_PER_TAB, nameFromUrl, totalFromRange, type Media } from "../src/sniffer";

describe("classify", () => {
  it("finds playlists and manifests by address or type", () => {
    expect(classify("https://a/live/index.m3u8?token=1", undefined, undefined)).toBe("hls");
    expect(classify("https://a/play", "application/vnd.apple.mpegurl; charset=utf-8", undefined)).toBe("hls");
    expect(classify("https://a/play", "application/x-mpegURL", 400)).toBe("hls");
    expect(classify("https://a/v/manifest.mpd", "text/xml", undefined)).toBe("dash");
    expect(classify("https://a/m", "application/dash+xml", undefined)).toBe("dash");
  });

  it("lists whole media files but not small ones or stream pieces", () => {
    expect(classify("https://a/v.mp4", "video/mp4", 5_000_000)).toBe("file");
    expect(classify("https://a/stream?id=1", "audio/mpeg", undefined)).toBe("file");
    expect(classify("https://a/v.mp4", "application/octet-stream", 5_000_000)).toBe("file");
    expect(classify("https://a/click.mp3", "audio/mpeg", 4_000)).toBeNull();
    expect(classify("https://a/seg1.ts", "video/mp2t", 900_000)).toBeNull();
    expect(classify("https://a/seg1.m4s", "video/iso.segment", 900_000)).toBeNull();
    expect(classify("https://a/seg1.ts", undefined, 900_000)).toBeNull();
  });

  it("ignores everything else", () => {
    expect(classify("https://a/app.js", "text/javascript", 10_000_000)).toBeNull();
    expect(classify("https://a/page.html", "text/html", undefined)).toBeNull();
    expect(classify("https://a/x.mp4", "text/html", undefined)).toBeNull();
    expect(classify("blob:https://a/1", "video/mp4", undefined)).toBeNull();
    expect(classify("not a url", "video/mp4", undefined)).toBeNull();
  });
});

describe("helpers", () => {
  it("reads the total size from Content-Range", () => {
    expect(totalFromRange("bytes 0-99/12345")).toBe(12345);
    expect(totalFromRange("bytes 0-99/*")).toBeUndefined();
    expect(totalFromRange(undefined)).toBeUndefined();
  });

  it("names media from the address", () => {
    expect(nameFromUrl("https://a/videos/Lesson%201.mp4")).toBe("Lesson 1");
    expect(nameFromUrl("https://a/course-7/master.m3u8")).toBe("course-7");
    expect(nameFromUrl("https://a/x/y/index.m3u8?t=1")).toBe("y");
    expect(nameFromUrl("https://a/")).toBe("video");
    expect(nameFromUrl("https://a/a%3Ab%2Fc.mp4")).toBe("a b c");
    expect(nameFromUrl("nonsense")).toBe("video");
  });

  it("keeps a bounded list, newest first, without duplicates", () => {
    const m = (n: number): Media => ({ url: `https://a/${n}.mp4`, kind: "file", name: String(n) });
    let list: Media[] = [];
    for (let i = 0; i < MAX_PER_TAB + 10; i++) list = addMedia(list, m(i));
    expect(list).toHaveLength(MAX_PER_TAB);
    expect(list[0]?.name).toBe(String(MAX_PER_TAB + 9));
    list = addMedia(list, m(MAX_PER_TAB));
    expect(list[0]?.name).toBe(String(MAX_PER_TAB));
    expect(list.filter((x) => x.name === String(MAX_PER_TAB))).toHaveLength(1);
  });
});

describe("segments and chunks are not files", () => {
  it("skips DASH/CMAF segments, numbered chunks and ranged player chunks", async () => {
    const { classify } = await import("../src/sniffer");
    expect(classify("https://v.bbc.test/a/video-1080/segment-12.m4s", "video/mp4", 900_000)).toBeNull();
    expect(classify("https://v.test/x/init.mp4", "video/mp4", 900_000)).toBe("file");
    expect(classify("https://v.test/x/chunk_00042.mp4", "video/mp4", 900_000)).toBeNull();
    expect(classify("https://r1.googlevideo.com/videoplayback?id=1&range=0-999", "video/mp4", 900_000)).toBeNull();
    expect(classify("https://v.test/film.mp4", "video/mp4", 90_000_000)).toBe("file");
  });
});
