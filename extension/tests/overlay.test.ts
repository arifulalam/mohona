// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { allowedOn, bigEnough, directSource, place, withHidden } from "../src/overlay";
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
    expect(place({ left: 100, top: 100, right: 740, bottom: 460 }, view)).toEqual({ left: 636, top: 108 });
    expect(place({ left: 100, top: 100, right: 1400, bottom: 460 }, view)?.left).toBe(896);
    expect(place({ left: 100, top: -300, right: 740, bottom: 60 }, view)).toBeNull();
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
