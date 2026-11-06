import { describe, expect, it } from "vitest";
import { isVideoPage, pageVideo } from "../src/videopage";

describe("video pages", () => {
  it("takes single video pages of known sites only", () => {
    for (const yes of [
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "https://m.youtube.com/watch?v=abc",
      "https://youtu.be/dQw4w9WgXcQ",
      "https://www.youtube.com/shorts/abc123",
      "https://vimeo.com/76979871",
      "https://www.dailymotion.com/video/x7tgad0",
    ]) expect(isVideoPage(yes), yes).toBe(true);
    for (const no of [
      "https://www.youtube.com/",
      "https://www.youtube.com/@someone",
      "https://www.youtube.com/results?search_query=x",
      "https://www.youtube.com/playlist?list=PL1",
      "https://vimeo.com/channels/staffpicks",
      "https://example.com/watch?v=abc",
      "https://youtube.com.evil.example/watch?v=abc",
      "ftp://youtube.com/watch?v=abc",
      "not a url",
      "",
    ]) expect(isVideoPage(no), no).toBe(false);
    expect(isVideoPage(undefined)).toBe(false);
  });

  it("offers the page as a download, named after its title", () => {
    const media = pageVideo("https://www.youtube.com/watch?v=abc", "(3) A fine talk - YouTube");
    expect(media).toEqual({ url: "https://www.youtube.com/watch?v=abc", kind: "file", name: "A fine talk", referrer: "https://www.youtube.com/watch?v=abc" });
    expect(pageVideo("https://www.youtube.com/", "x")).toBeNull();
    expect(pageVideo("https://vimeo.com/123", "")?.name).toBe("This video");
  });
});

describe("pageGuess", () => {
  it("offers other web pages, but not files, known video pages, channels or non-web addresses", async () => {
    const { pageGuess } = await import("../src/videopage");
    for (const yes of [
      "https://www.facebook.com/watch/?v=1",
      "https://x.com/a/status/1",
      "https://www.instagram.com/reel/abc/",
      "https://soundcloud.com/a/b",
      "https://example.com/page.html",
    ]) {
      const m = pageGuess(yes);
      expect(m?.url, yes).toBe(yes);
      expect(m?.video).toBe(true);
    }
    for (const no of [
      "https://www.youtube.com/watch?v=abc",
      "https://www.youtube.com/@someone",
      "https://example.com/file.zip",
      "chrome://extensions",
      "about:blank",
      undefined,
    ]) {
      expect(pageGuess(no as string), String(no)).toBeNull();
    }
  });
});
