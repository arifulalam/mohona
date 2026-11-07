import { describe, expect, it } from "vitest";
import { filesOf, groupOf, type RawLink } from "../src/pagelinks";

const page = "https://example.com/gallery/";

describe("filesOf", () => {
  it("keeps files, drops pages and the web's own machinery, once each and in page order", () => {
    const links: RawLink[] = [
      { url: "https://example.com/gallery/a.pdf", kind: "a" },
      { url: "https://example.com/gallery/next.html", kind: "a" },
      { url: "https://example.com/gallery/more", kind: "a" },
      { url: "https://example.com/style.css", kind: "a" },
      { url: "https://example.com/p.JPG#x", kind: "media" },
      { url: "https://example.com/p.JPG", kind: "media" },
      { url: "https://example.com/raw", kind: "download" },
      { url: "mailto:x@y.z", kind: "a" },
      { url: "javascript:void(0)", kind: "a" },
      { url: "https://example.com/gallery/", kind: "a" },
      { url: "https://example.com/My%20Song.mp3?x=1", kind: "a" },
      { url: "not a url", kind: "a" },
    ];
    expect(filesOf(links, page).map((f) => [f.name, f.group])).toEqual([
      ["a.pdf", "documents"],
      ["p.JPG", "images"],
      ["raw", "other"],
      ["My Song.mp3", "audio"],
    ]);
  });

  it("names the groups", () => {
    expect(groupOf("zip")).toBe("archives");
    expect(groupOf("mp4")).toBe("video");
    expect(groupOf("")).toBe("other");
  });
});
