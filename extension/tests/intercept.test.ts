import { describe, expect, it } from "vitest";
import { cookieHeaderOf, intercept, skipReason, type Deps } from "../src/intercept";
import { DEFAULTS, hostMatches, sanitize, type Settings } from "../src/settings";
import type { Outcome } from "../src/protocol";

function deps(outcome: Outcome | Error, settings: Settings = DEFAULTS, cookies = "a=b") {
  const calls: string[] = [];
  const d: Deps = {
    settings,
    userAgent: "UA",
    downloads: {
      pause: async (id) => void calls.push(`pause ${id}`),
      resume: async (id) => void calls.push(`resume ${id}`),
      cancel: async (id) => void calls.push(`cancel ${id}`),
      erase: async (id) => void calls.push(`erase ${id}`),
    },
    cookieHeader: async () => cookies,
    add: async (details) => {
      calls.push(`add ${details.url} cookies=${details.cookies ?? "-"} ref=${details.referrer ?? "-"}`);
      if (outcome instanceof Error) throw outcome;
      return outcome;
    },
  };
  return { d, calls };
}

const item = { id: 7, url: "https://example.com/f.zip", referrer: "https://example.com/" };

describe("intercept", () => {
  it("hands the download over, then cancels and erases the browser's copy", async () => {
    const { d, calls } = deps({ kind: "added", id: 1 });
    expect(await intercept(item, d)).toEqual({ kind: "handed", url: item.url });
    expect(calls).toEqual([
      "pause 7",
      "add https://example.com/f.zip cookies=a=b ref=https://example.com/",
      "cancel 7",
      "erase 7",
    ]);
  });

  it("uses the final address after redirects", async () => {
    const { d, calls } = deps({ kind: "added", id: 1 });
    await intercept({ ...item, finalUrl: "https://cdn.example.com/x.zip" }, d);
    expect(calls[1]).toContain("add https://cdn.example.com/x.zip");
  });

  it("fails open: the download resumes when Mohona cannot take it", async () => {
    for (const outcome of [
      { kind: "error", code: "no_engine", message: "not found" } as Outcome,
      { kind: "duplicate", message: "already" } as Outcome,
      new Error("host gone"),
    ]) {
      const { d, calls } = deps(outcome);
      const result = await intercept(item, d);
      expect(result.kind).toBe("kept");
      expect(calls.at(-1)).toBe("resume 7");
      expect(calls.some((c) => c.startsWith("cancel"))).toBe(false);
    }
  });

  it("still hands over without cookies when they cannot be read", async () => {
    const { d, calls } = deps({ kind: "added", id: 1 });
    d.cookieHeader = async () => {
      throw new Error("no permission");
    };
    expect((await intercept(item, d)).kind).toBe("handed");
    expect(calls[1]).toContain("cookies=-");
  });

  it("does not touch downloads it will not take", async () => {
    for (const [it, settings] of [
      [item, { ...DEFAULTS, intercept: false }],
      [{ ...item, incognito: true }, DEFAULTS],
      [{ ...item, url: "blob:https://example.com/1" }, DEFAULTS],
      [{ ...item, url: "data:text/plain,hi" }, DEFAULTS],
      [{ ...item, url: "file:///etc/passwd" }, DEFAULTS],
      [item, { ...DEFAULTS, excludedHosts: ["example.com"] }],
    ] as [typeof item, Settings][]) {
      const { d, calls } = deps({ kind: "added", id: 1 }, settings);
      expect((await intercept(it, d)).kind).toBe("skipped");
      expect(calls).toEqual([]);
    }
  });
});

describe("rules", () => {
  it("applies the size limit only when the size is known", () => {
    const settings = { ...DEFAULTS, minSizeBytes: 1000 };
    expect(skipReason({ ...item, fileSize: 10 }, settings)).toMatch(/smaller/);
    expect(skipReason({ ...item, fileSize: 5000 }, settings)).toBeNull();
    expect(skipReason({ ...item, fileSize: -1 }, settings)).toBeNull();
    expect(skipReason(item, settings)).toBeNull();
  });

  it("matches excluded sites and their subdomains, not look-alikes", () => {
    expect(hostMatches("example.com", "example.com")).toBe(true);
    expect(hostMatches("cdn.Example.com", "example.com")).toBe(true);
    expect(hostMatches("badexample.com", "example.com")).toBe(false);
    expect(hostMatches("example.com.evil.org", "example.com")).toBe(false);
  });

  it("builds a Cookie header", () => {
    expect(cookieHeaderOf([{ name: "a", value: "1" }, { name: "b", value: "2" }])).toBe("a=1; b=2");
    expect(cookieHeaderOf([])).toBe("");
  });

  it("makes stored settings safe", () => {
    expect(sanitize(undefined)).toEqual(DEFAULTS);
    expect(sanitize({ intercept: "yes", minSizeBytes: -5, excludedHosts: "x" })).toEqual(DEFAULTS);
    expect(sanitize({ intercept: false, minSizeBytes: 12.7, excludedHosts: [" *.Example.com ", "", 3, "a.org"] })).toEqual({
      intercept: false,
      minSizeBytes: 12,
      excludedHosts: ["example.com", "a.org"],
    });
  });
});
