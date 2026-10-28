import { describe, expect, it } from "vitest";
import { allowed, counts, matches } from "../src/filters";
import * as fmt from "../src/format";
import { parseHosts, parseLimit, parseMegabytes, splitLimit } from "../src/limits";
import { parseTypes } from "../src/CategoriesDialog";
import { describeWindow, normalTime, windowFrom } from "../src/schedule";
import { applyTheme, nextTheme } from "../src/theme";
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
  it("cycles and marks the page", () => {
    expect(nextTheme("system")).toBe("light");
    expect(nextTheme("light")).toBe("dark");
    expect(nextTheme("dark")).toBe("system");
    const el = document.createElement("html");
    applyTheme("dark", el);
    expect(el.getAttribute("data-theme")).toBe("dark");
    applyTheme("system", el);
    expect(el.hasAttribute("data-theme")).toBe(false);
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
