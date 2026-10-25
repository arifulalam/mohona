// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { linksInPage, MAX_TRIES, nextToRefresh, pickLink, refreshExpired, score, type RefreshDeps } from "../src/refresh";
import type { DownloadRow } from "../src/protocol";

const row = (over: Partial<DownloadRow> = {}): DownloadRow => ({
  id: 1,
  url: "https://cdn.example.com/files/lesson%201.mp4?sig=OLD&exp=1",
  filename: "lesson 1.mp4",
  status: "failed",
  downloaded: 0,
  size: null,
  error: "the server answered 403 for x",
  path: null,
  page: "https://example.com/course/1",
  expired: true,
  ...over,
});

describe("pickLink", () => {
  it("prefers the same file on the same site with a new signature", () => {
    const link = pickLink(row(), [
      "https://example.com/about",
      "https://cdn.example.com/files/other.mp4?sig=X",
      "https://cdn.example.com/files/lesson%201.mp4?sig=NEW&exp=2",
    ]);
    expect(link).toBe("https://cdn.example.com/files/lesson%201.mp4?sig=NEW&exp=2");
  });

  it("falls back to the same file name on another host", () => {
    expect(
      pickLink(row(), ["https://edge7.example.net/dl/Lesson%201.mp4?t=9", "https://example.com/"]),
    ).toBe("https://edge7.example.net/dl/Lesson%201.mp4?t=9");
  });

  it("never returns the link that already failed, or something unrelated", () => {
    expect(pickLink(row(), [row().url])).toBeNull();
    expect(pickLink(row(), ["https://example.com/", "mailto:a@b.c", "javascript:void(0)", "not a url"])).toBeNull();
    expect(pickLink(row(), [])).toBeNull();
  });

  it("uses the file name when the old address had none", () => {
    const old = row({ url: "https://example.com/get?id=5&sig=OLD", filename: "report.pdf" });
    expect(pickLink(old, ["https://example.com/files/report.pdf?sig=NEW"])).toBe(
      "https://example.com/files/report.pdf?sig=NEW",
    );
  });

  it("scores in the right order", () => {
    const old = "https://a.com/x/f.zip?s=1";
    expect(score(old, "https://a.com/x/f.zip?s=2")).toBe(3);
    expect(score(old, "https://b.com/y/F.zip?s=2")).toBe(2);
    expect(score(old, "https://a.com/z/g.zip")).toBe(0);
    expect(score(old, old)).toBe(0);
  });
});

describe("refreshExpired", () => {
  const deps = (links: string[], answer: string | null = null) => {
    const calls: unknown[][] = [];
    const d: RefreshDeps = {
      collect: async () => links,
      cookieHeader: async () => "s=1",
      userAgent: "UA",
      refresh: async (...args) => {
        calls.push(args);
        return answer;
      },
    };
    return { d, calls };
  };

  it("sends the chosen link with the page as referrer", async () => {
    const { d, calls } = deps(["https://cdn.example.com/files/lesson%201.mp4?sig=NEW"]);
    expect(await refreshExpired(row(), d)).toEqual({ kind: "refreshed" });
    expect(calls[0]).toEqual([1, "https://cdn.example.com/files/lesson%201.mp4?sig=NEW", "s=1", "https://example.com/course/1", "UA"]);
  });

  it("says what went wrong", async () => {
    expect(await refreshExpired(row({ page: null }), deps([]).d)).toEqual({ kind: "no_page" });
    expect(await refreshExpired(row(), deps(["https://example.com/"]).d)).toEqual({ kind: "no_link" });
    const refused = await refreshExpired(
      row(),
      deps(["https://cdn.example.com/files/lesson%201.mp4?sig=NEW"], "different file").d,
    );
    expect(refused).toEqual({ kind: "refused", message: "different file" });
    const broken: RefreshDeps = { ...deps([]).d, collect: async () => { throw new Error("tab closed"); } };
    expect(await refreshExpired(row(), broken)).toEqual({ kind: "failed", message: "tab closed" });
  });
});

describe("nextToRefresh", () => {
  it("takes failed, expired downloads with a page, a limited number of times", () => {
    const rows = [row({ id: 1, expired: false }), row({ id: 2, page: null }), row({ id: 3, status: "paused" }), row({ id: 4 }), row({ id: 5 })];
    expect(nextToRefresh(rows, new Map())?.id).toBe(4);
    expect(nextToRefresh(rows, new Map([[4, MAX_TRIES]]))?.id).toBe(5);
    expect(nextToRefresh(rows, new Map([[4, MAX_TRIES], [5, MAX_TRIES]]))).toBeUndefined();
  });
});

describe("linksInPage", () => {
  it("collects addresses from links and media, made absolute and without duplicates", () => {
    document.head.innerHTML = '<base href="https://example.com/course/">';
    document.body.innerHTML = `
      <a href="/dl/a.zip?s=1">a</a><a href="/dl/a.zip?s=1">again</a><a href="mailto:x@y.z">mail</a>
      <video src="v.mp4"></video><video><source src="https://cdn.example.com/s.m3u8"></video><a>no href</a>`;
    const links = linksInPage();
    expect(links).toContain("https://example.com/dl/a.zip?s=1");
    expect(links).toContain("https://example.com/course/v.mp4");
    expect(links).toContain("https://cdn.example.com/s.m3u8");
    expect(links.filter((l) => l.endsWith("a.zip?s=1")).length).toBe(1);
  });
});
