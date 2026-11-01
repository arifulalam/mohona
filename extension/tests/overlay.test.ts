// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { allowedOn, bigEnough, directSource, place, placeBeside, playersToMark, withHidden } from "../src/overlay";
import { DEFAULTS, sanitize } from "../src/settings";

describe("overlay", () => {
  it("ignores small players", () => {
    expect(bigEnough({ width: 640, height: 360 })).toBe(true);
    expect(bigEnough({ width: 200, height: 100 })).toBe(false);
  });

  it("follows the global switch and the per-site list", () => {
    const on = { videoButton: true, buttonHiddenHosts: ["example.com"] };
    expect(allowedOn(on, "other.org")).toBe(true);
    expect(allowedOn(on, "example.com")).toBe(false);
    expect(allowedOn(on, "video.example.com")).toBe(false);
    expect(allowedOn({ ...on, videoButton: false }, "other.org")).toBe(false);
  });

  it("adds a hidden site once, without www", () => {
    expect(withHidden([], "www.Example.com")).toEqual(["example.com"]);
    expect(withHidden(["example.com"], "example.com")).toEqual(["example.com"]);
  });

  it("puts the button in the top right corner and keeps it in view", () => {
    const view = { width: 1000, height: 800 };
    expect(place({ left: 100, top: 100, right: 740, bottom: 460 }, view)).toEqual({ left: 696, top: 108 });
    expect(place({ left: 100, top: 100, right: 1400, bottom: 460 }, view)?.left).toBe(956);
    expect(place({ left: 100, top: -300, right: 740, bottom: 60 }, view)).toBeNull();
  });

  it("marks big videos and sound players that show controls, not hidden or tiny ones", () => {
    const box = (el: HTMLElement, width: number, height: number) => {
      el.getBoundingClientRect = () => ({ width, height, left: 0, top: 0, right: width, bottom: height, x: 0, y: 0, toJSON() {} }) as DOMRect;
      document.body.append(el);
      return el;
    };
    document.body.replaceChildren();
    const big = box(document.createElement("video"), 640, 360);
    box(document.createElement("video"), 100, 60);
    const withControls = document.createElement("audio");
    withControls.controls = true;
    box(withControls, 300, 40);
    box(document.createElement("audio"), 300, 40); // no controls: hidden machinery
    const thin = document.createElement("audio");
    thin.controls = true;
    box(thin, 120, 40);
    expect(playersToMark(document)).toEqual([big, withControls]);
  });

  it("keeps a low sound player's button in view", () => {
    const view = { width: 1000, height: 800 };
    expect(place({ left: 100, top: 100, right: 500, bottom: 140 }, view)).toBeNull();
    expect(place({ left: 100, top: 100, right: 500, bottom: 140 }, view, undefined, { width: 80, height: 20 })).toEqual({ left: 456, top: 108 });
  });

  it("puts a sound player's button beside it, above it, or in its corner", () => {
    const view = { width: 1000, height: 800 };
    const bar = { left: 100, top: 100, right: 500, bottom: 140 };
    expect(placeBeside(bar, view)).toEqual({ left: 506, top: 104 });
    expect(placeBeside({ ...bar, right: 990 }, view)).toEqual({ left: 946, top: 64 });
    expect(placeBeside({ left: 100, top: 10, right: 990, bottom: 50 }, view)).toEqual({ left: 946, top: 18 });
    expect(placeBeside({ left: 100, top: 900, right: 500, bottom: 940 }, view)).toBeNull();
  });

  it("only offers real addresses as a direct source", () => {
    const video = document.createElement("video");
    video.setAttribute("src", "blob:https://x/1");
    expect(directSource(video)).toBeNull();
    video.setAttribute("src", "https://x.test/a.mp4");
    expect(directSource(video)).toBe("https://x.test/a.mp4");
  });

  it("settings keep the new fields clean", () => {
    expect(DEFAULTS.videoButton).toBe(true);
    const s = sanitize({ videoButton: false, buttonHiddenHosts: [" WWW.a.com", 5, "*.b.org"] });
    expect(s.videoButton).toBe(false);
    expect(s.buttonHiddenHosts).toEqual(["www.a.com", "b.org"]);
  });
});
