import { describe, expect, it } from "vitest";
import { BUTTON_TEXTS, colorsFor, lookFor } from "../src/look";

describe("the look of the button on pages", () => {
  it("speaks the chosen language, with only the words it uses", () => {
    const look = lookFor("forest", "bn", ["fr"]);
    expect(look.lang).toBe("bn");
    expect(look.words["Download with Mohona"]).toBeTruthy();
    expect(look.words["Hide on {site}"]).toContain("{site}");
    expect(Object.keys(look.words).every((k) => BUTTON_TEXTS.includes(k))).toBe(true);
  });

  it("follows the browser's language when none was chosen, and English when unknown", () => {
    expect(lookFor(null, null, ["de-AT"]).lang).toBe("de");
    const english = lookFor(null, null, ["xx"]);
    expect(english.lang).toBe("en");
    expect(english.words).toEqual({});
  });

  it("is right to left for Arabic", () => {
    expect(lookFor(null, "ar", []).dir).toBe("rtl");
    expect(lookFor(null, "es", []).dir).toBe("ltr");
  });

  it("takes the colours of the chosen theme, and none for the system's", () => {
    expect(colorsFor("system")).toBeNull();
    expect(colorsFor(null)).toBeNull();
    expect(colorsFor("nonsense")).toBeNull();
    expect(colorsFor("dark")?.dark).toBe(true);
    expect(colorsFor("light")?.dark).toBe(false);
    const river = colorsFor("river")!;
    expect(river.dark).toBe(true);
    expect(river.hero).toContain("linear-gradient");
    expect(river.accentText).toMatch(/^#/);
  });
});
