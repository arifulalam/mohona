import { describe, expect, it } from "vitest";
import { AUDIO, CONTAINERS, cleanChoice, DEFAULT_CHOICE, formatsFor } from "../src/videochoice";

describe("video choice", () => {
  it("keeps what the engine knows and defaults the rest", () => {
    expect(cleanChoice(null)).toEqual(DEFAULT_CHOICE);
    expect(cleanChoice({ quality: "720", container: "mkv", audio: "flac" })).toEqual({ quality: "720", container: "mkv", audio: "flac" });
    expect(cleanChoice({ quality: "9999", container: "exe", audio: "<b>" })).toEqual(DEFAULT_CHOICE);
  });

  it("offers sound formats for sound only and video formats otherwise", () => {
    expect(formatsFor("audio")).toBe(AUDIO);
    expect(formatsFor("1080")).toBe(CONTAINERS);
  });
});
