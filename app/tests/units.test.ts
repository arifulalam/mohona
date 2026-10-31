import { describe, expect, it } from "vitest";
import { allowed, counts, matches } from "../src/filters";
import * as fmt from "../src/format";
import { parseHosts, parseLimit, parseMegabytes, splitLimit } from "../src/limits";
import { parseTypes } from "../src/CategoriesDialog";
import { describeWindow, normalTime, windowFrom } from "../src/schedule";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { applyTheme, THEMES } from "../src/theme";
import { linkIn } from "../src/clipboard";
import { categoryCounts, categoryOf, extensionOf, sortRows } from "../src/organize";
import type { Row } from "../src/types";

const row = (id: number, status: string): Row => ({
  id, name: `f${id}`, url: "https://x.test/" + id, status, downloaded: 0, size: null, percent: null,
  speed: 0, eta: null, error: null, path: null, expired: false,
});

describe("format", () => {
  it("writes sizes, speeds and times people can read", () => {
    expect(fmt.bytes(0)).toBe("0 B");
    expect(fmt.bytes(1536)).toBe("1.5 KB");
    expect(fmt.bytes(150 * 1024 * 1024)).toBe("150 MB");
    expect(fmt.speed(0)).toBe("");
    expect(fmt.speed(2048)).toBe("2.0 KB/s");
    expect(fmt.eta(null)).toBe("");
    expect(fmt.eta(45)).toBe("45s");
    expect(fmt.eta(75)).toBe("1m 15s");
    expect(fmt.eta(3700)).toBe("1h 1m");
    expect(fmt.eta(200000)).toBe("2d");
  });
});

describe("filters", () => {
  const rows = [row(1, "downloading"), row(2, "paused"), row(3, "completed"), row(4, "failed"), row(5, "canceled"), row(6, "queued")];
  it("sorts downloads into the groups of the sidebar", () => {
    expect(counts(rows)).toEqual({ all: 6, active: 3, finished: 1, problems: 2 });
    expect(rows.filter((r) => matches(r, "problems")).map((r) => r.id)).toEqual([4, 5]);
  });
  it("allows only what makes sense for the selection", () => {
    expect(allowed([])).toEqual({ pause: false, resume: false, cancel: false, remove: false });
    expect(allowed([row(1, "downloading")])).toEqual({ pause: true, resume: false, cancel: true, remove: true });
    expect(allowed([row(2, "paused")])).toEqual({ pause: false, resume: true, cancel: true, remove: true });
    expect(allowed([row(3, "completed")])).toEqual({ pause: false, resume: false, cancel: false, remove: true });
    expect(allowed([row(1, "downloading"), row(4, "failed")])).toMatchObject({ pause: true, resume: true });
  });
});

describe("theme", () => {
  it("marks the page, and system leaves it to the operating system", () => {
    const el = document.createElement("html");
    applyTheme("dark", el);
    expect(el.getAttribute("data-theme")).toBe("dark");
    applyTheme("forest", el);
    expect(el.getAttribute("data-theme")).toBe("forest");
    applyTheme("system", el);
    expect(el.hasAttribute("data-theme")).toBe(false);
  });
  it("has a colour block in the style sheet for every named theme", () => {
    const css = readFileSync(join(process.cwd(), "src", "themes.css"), "utf8");
    const named = THEMES.filter((t) => !["system", "light", "dark"].includes(t.id));
    expect(named.map((t) => t.id)).toEqual(["desert", "forest", "sunny", "orange", "leaf", "earth", "sky", "river", "water"]);
    for (const t of named) expect(css).toContain(`:root[data-theme="${t.id}"]`);
  });
});

describe("categories and sorting", () => {
  const cats = [
    { id: 1, name: "Compressed", extensions: ["zip", "7z"], folder: null },
    { id: 2, name: "Video", extensions: ["mp4"], folder: null },
  ];
  const r = (id: number, name: string, status = "completed", size: number | null = 100): Row => ({ ...row(id, status), name, size, downloaded: size ?? 0 });
  it("finds a file's category by its extension, case-blind, else Other", () => {
    expect(categoryOf("a.ZIP", cats)).toBe("Compressed");
    expect(categoryOf("clip.mp4", cats)).toBe("Video");
    expect(categoryOf("notes.txt", cats)).toBe("Other");
    expect(categoryOf("README", cats)).toBe("Other");
    expect(categoryOf(".zip", cats)).toBe("Other");
    expect(extensionOf("a.tar.gz")).toBe("gz");
  });
  it("counts rows per category and hides an empty Other", () => {
    const rows = [r(1, "a.zip"), r(2, "b.7z"), r(3, "c.mp4")];
    expect(categoryCounts(rows, cats)).toEqual([{ name: "Compressed", count: 2 }, { name: "Video", count: 1 }]);
    expect(categoryCounts([...rows, r(4, "x.bin")], cats).at(-1)).toEqual({ name: "Other", count: 1 });
  });
  it("sorts by name, size, category and status without losing rows", () => {
    const rows = [r(1, "b.zip", "completed", 5), r(2, "a.mp4", "downloading", 50), r(3, "c.txt", "failed", 20), r(4, "a.zip", "queued", 20)];
    const names = (k: Parameters<typeof sortRows>[1]) => sortRows(rows, k, cats).map((x) => x.name);
    expect(names("added")).toEqual(["a.zip", "c.txt", "a.mp4", "b.zip"]);
    expect(names("name")).toEqual(["a.mp4", "a.zip", "b.zip", "c.txt"]);
    expect(names("size")).toEqual(["a.mp4", "a.zip", "c.txt", "b.zip"]);
    expect(names("type")).toEqual(["a.zip", "b.zip", "c.txt", "a.mp4"]);
    expect(names("status")).toEqual(["a.mp4", "a.zip", "c.txt", "b.zip"]);
    expect(sortRows(rows, "name", cats)).toHaveLength(4);
    expect(rows[0]?.name).toBe("b.zip");
  });
});

describe("limits", () => {
  it("reads what was typed", () => {
    expect(parseLimit("", "MB")).toBe(0);
    expect(parseLimit("2", "MB")).toBe(2 * 1024 * 1024);
    expect(parseLimit("1,5", "KB")).toBe(1536);
    expect(parseLimit("abc", "MB")).toBeNull();
    expect(parseLimit("-1", "MB")).toBeNull();
    expect(parseMegabytes("5")).toBe(5 * 1024 * 1024);
    expect(parseMegabytes("5.5")).toBeNull();
    expect(parseHosts(" a.com \n\n b.org\r\n")).toEqual(["a.com", "b.org"]);
  });
  it("shows a limit in the unit that reads best", () => {
    expect(splitLimit(null)).toEqual({ text: "", unit: "MB" });
    expect(splitLimit(3 * 1024 * 1024)).toEqual({ text: "3", unit: "MB" });
    expect(splitLimit(1536)).toEqual({ text: "1.5", unit: "KB" });
  });
});

describe("schedule", () => {
  it("reads times and builds windows", () => {
    expect(normalTime("7:30")).toBe("07:30");
    expect(normalTime("24:00")).toBeNull();
    expect(normalTime("abc")).toBeNull();
    expect(windowFrom(["mon", "tue"], "23:00", "6:00", "")).toEqual({ days: ["mon", "tue"], start: "23:00", end: "06:00", speedLimit: null });
    expect(windowFrom(["sun", "mon", "tue", "wed", "thu", "fri", "sat"], "1:00", "2:00", "512")).toEqual({ days: [], start: "01:00", end: "02:00", speedLimit: 524288 });
    expect(windowFrom([], "10:00", "10:00", "")).toMatch(/same time/);
    expect(windowFrom([], "10:00", "11:00", "x")).toMatch(/number/);
    expect(describeWindow({ days: ["sat", "sun"], start: "00:00", end: "08:00", speedLimit: 1024 })).toBe("Sat, Sun 00:00–08:00, up to 1 KB/s");
  });
});

describe("categories", () => {
  it("reads file types as typed", () => {
    expect(parseTypes("zip, .7z  RAR;tar")).toEqual(["zip", "7z", "rar", "tar"]);
    expect(parseTypes("  ")).toEqual([]);
  });
});

describe("copied links", () => {
  it("accepts one whole link and nothing else", () => {
    expect(linkIn(" https://example.com/a.zip \n")).toBe("https://example.com/a.zip");
    expect(linkIn("ftp://files.example.com/x.iso")).toBe("ftp://files.example.com/x.iso");
    for (const bad of [null, "", "hello", "see https://example.com/a.zip", "javascript:alert(1)", "file:///etc/passwd", "http://localhost/a", "https://example.com/" + "a".repeat(3000)]) {
      expect(linkIn(bad)).toBeNull();
    }
  });
});
