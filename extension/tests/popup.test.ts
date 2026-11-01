import { describe, expect, it } from "vitest";
import { ALL_THEMES, applyTheme, loadTheme, saveTheme } from "../src/popuptheme";
import type { DownloadRow } from "../src/protocol";
import { brief, bytes, eta, nameOf, speed, speeds, summarize } from "../src/summary";

const row = (id: number, status: string, downloaded = 0, extra: Partial<DownloadRow> = {}): DownloadRow => ({
  id, url: `https://x.test/f${id}.zip`, filename: null, status, downloaded, size: 1000, error: null, path: null, page: null, expired: false, ...extra,
});

describe("popup summary", () => {
  it("counts running, waiting, finished and failed downloads", () => {
    const s = summarize([row(1, "downloading"), row(2, "probing"), row(3, "queued"), row(4, "paused"), row(5, "completed"), row(6, "failed"), row(7, "canceled")]);
    expect(s).toEqual({ running: 2, waiting: 2, finished: 1, failed: 2 });
  });
  it("measures speed between two lists and ignores restarts and stopped rows", () => {
    const before = new Map([[1, 100], [2, 500], [3, 10]]);
    const out = speeds(before, [row(1, "downloading", 600), row(2, "downloading", 100), row(3, "paused", 50), row(4, "downloading", 5)], 2);
    expect(out.get(1)).toBe(250);
    expect(out.has(2)).toBe(false);
    expect(out.has(3)).toBe(false);
    expect(out.has(4)).toBe(false);
    expect(speeds(before, [row(1, "downloading", 600)], 0).size).toBe(0);
  });
  it("formats sizes, speeds and times", () => {
    expect(bytes(1536)).toBe("1.5 KB");
    expect(bytes(null)).toBe("");
    expect(speed(2 * 1024 * 1024)).toBe("2.0 MB/s");
    expect(eta(42)).toBe("42 s");
    expect(eta(125)).toBe("2 min 5 s");
    expect(eta(3 * 3600 + 120)).toBe("3 h 2 min");
    expect(eta(Number.NaN)).toBe("");
  });
  it("names a download by its file name, else its address", () => {
    expect(nameOf(row(1, "queued", 0, { filename: "a.zip" }))).toBe("a.zip");
    expect(nameOf(row(1, "queued", 0, { url: "https://x.test/dir/my%20file.pdf?x=1" }))).toBe("my file.pdf");
    expect(nameOf(row(1, "queued", 0, { url: "https://x.test/" }))).toBe("x.test");
  });
});

describe("popup theme", () => {
  it("offers the system, light, dark and the named themes", () => {
    const ids = ALL_THEMES.map((t) => t.id);
    expect(ids.slice(0, 3)).toEqual(["system", "light", "dark"]);
    for (const name of ["desert", "forest", "sunny", "orange", "leaf", "earth", "sky", "river", "water"]) expect(ids).toContain(name);
  });
  it("remembers a known theme and falls back for anything else", () => {
    const data = new Map<string, string>();
    const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
    expect(loadTheme(storage)).toBe("system");
    saveTheme(storage, "forest");
    expect(loadTheme(storage)).toBe("forest");
    saveTheme(storage, "<script>");
    expect(loadTheme(storage)).toBe("system");
    expect(loadTheme(null)).toBe("system");
    expect(() => saveTheme({ setItem: () => { throw new Error("blocked"); } }, "dark")).not.toThrow();
  });
  it("sets and clears the page's theme", () => {
    const attributes = new Map<string, string>();
    const el = {
      setAttribute: (k: string, v: string) => void attributes.set(k, v),
      removeAttribute: (k: string) => void attributes.delete(k),
    } as unknown as HTMLElement;
    applyTheme(el, "river");
    expect(attributes.get("data-theme")).toBe("river");
    applyTheme(el, "system");
    expect(attributes.has("data-theme")).toBe(false);
  });
});

describe("brief", () => {
  it("shortens long addresses and leaves short text alone", async () => {
    
    const long = "https://javacodegeeks.tradepub.com/?p=w_wile925&w=d&email=f9677dca1d4a296be107129d82eaaa20&key=vvhhOX6xX2zMxcHscp5Z&ts=82103";
    const out = brief(`Could not refresh the link of ${long}: 403`);
    expect(out).toContain("…");
    expect(out.length).toBeLessThan(100);
    expect(brief("Added to Mohona.")).toBe("Added to Mohona.");
    expect(brief("https://a.example/x")).toBe("https://a.example/x");
  });
});
