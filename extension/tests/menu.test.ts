import { describe, expect, it } from "vitest";
import { downloadFromMenu, targetOf, type MenuDeps } from "../src/menu";
import type { AddDetails, Outcome } from "../src/protocol";

function deps(outcome: Outcome | Error) {
  const sent: AddDetails[] = [];
  const d: MenuDeps = {
    userAgent: "UA",
    cookieHeader: async () => "s=1",
    add: async (details) => {
      sent.push(details);
      if (outcome instanceof Error) throw outcome;
      return outcome;
    },
  };
  return { d, sent };
}

describe("targetOf", () => {
  it("prefers the link, then the media source", () => {
    expect(targetOf({ linkUrl: "https://a/l.zip", srcUrl: "https://a/i.png" })).toBe("https://a/l.zip");
    expect(targetOf({ srcUrl: "https://a/v.mp4" })).toBe("https://a/v.mp4");
    expect(targetOf({ srcUrl: "ftp://a/v.mp4" })).toBe("ftp://a/v.mp4");
    expect(targetOf({ linkUrl: "magnet:?xt=urn:btih:abc&dn=x" })).toBe("magnet:?xt=urn:btih:abc&dn=x");
  });
  it("refuses what cannot be downloaded", () => {
    expect(targetOf({})).toBeNull();
    expect(targetOf({ srcUrl: "blob:https://a/1" })).toBeNull();
    expect(targetOf({ linkUrl: "javascript:alert(1)" })).toBeNull();
    expect(targetOf({ linkUrl: "mailto:a@b.c" })).toBeNull();
    expect(targetOf({ srcUrl: "not a url" })).toBeNull();
  });
});

describe("downloadFromMenu", () => {
  it("sends the address with the page as referrer and the site's cookies", async () => {
    const { d, sent } = deps({ kind: "added", id: 3 });
    expect(await downloadFromMenu({ linkUrl: "https://a/f.zip", pageUrl: "https://a/page" }, d)).toEqual({
      kind: "sent",
      url: "https://a/f.zip",
    });
    expect(sent[0]).toEqual({ url: "https://a/f.zip", referrer: "https://a/page", cookies: "s=1", userAgent: "UA" });
  });

  it("uses the frame as referrer when the link is inside one", async () => {
    const { d, sent } = deps({ kind: "added", id: 3 });
    await downloadFromMenu({ linkUrl: "https://a/f", pageUrl: "https://a/", frameUrl: "https://b/embed" }, d);
    expect(sent[0]?.referrer).toBe("https://b/embed");
  });

  it("says why it failed, including a duplicate", async () => {
    expect(
      await downloadFromMenu({ linkUrl: "https://a/f" }, deps({ kind: "duplicate", message: "in the list" }).d),
    ).toEqual({ kind: "failed", message: "Mohona already has it: in the list" });
    expect(
      await downloadFromMenu({ linkUrl: "https://a/f" }, deps({ kind: "error", code: "x", message: "no engine" }).d),
    ).toEqual({ kind: "failed", message: "no engine" });
    expect(await downloadFromMenu({ linkUrl: "https://a/f" }, deps(new Error("gone")).d)).toEqual({
      kind: "failed",
      message: "gone",
    });
  });

  it("ignores clicks without an address and sends nothing", async () => {
    const { d, sent } = deps({ kind: "added", id: 1 });
    expect((await downloadFromMenu({ pageUrl: "https://a/" }, d)).kind).toBe("ignored");
    expect(sent).toEqual([]);
  });
});
