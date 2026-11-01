import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fill, label, LANGUAGES, pickLanguage, translator } from "../src/i18n";

const root = process.cwd();
const locales = join(root, "src", "locales");
const files = readdirSync(locales).filter((n) => /^[a-z].*\.json$/.test(n));

/** Every text the popup can show: t("..."), the page's data-t attributes, the labels, and the theme names. */
function texts(): string[] {
  const found = new Set<string>();
  const ts = readFileSync(join(root, "src", "popup.ts"), "utf8");
  for (const m of ts.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) found.add(JSON.parse(`"${m[1]}"`));
  const html = readFileSync(join(root, "static", "popup.html"), "utf8");
  for (const m of html.matchAll(/data-t(?:-label|-placeholder)?="([^"]+)"/g)) found.add(m[1]!);
  const label = ts.match(/const LABEL[^{]*\{([^}]*)\}/)?.[1] ?? "";
  for (const m of label.matchAll(/:\s*"([^"]+)"/g)) found.add(m[1]!);
  for (const m of readFileSync(join(root, "src", "themes.generated.ts"), "utf8").matchAll(/"label": "([^"]+)"/g)) found.add(m[1]!);
  for (const name of ["System", "Light", "Dark"]) found.add(name);
  const look = readFileSync(join(root, "src", "look.ts"), "utf8");
  for (const m of (look.match(/BUTTON_TEXTS = \[([\s\S]*?)\];/)?.[1] ?? "").matchAll(/"((?:[^"\\]|\\.)*)"/g)) found.add(JSON.parse(`"${m[1]}"`));
  return [...found];
}

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");

describe("popup languages", () => {
  it("lists English first and every language file, with both names", () => {
    expect(LANGUAGES[0]?.code).toBe("en");
    expect(LANGUAGES.map((l) => l.code).sort()).toEqual(["en", ...files.map((f) => f.replace(".json", ""))].sort());
    expect(label(LANGUAGES.find((l) => l.code === "bn")!)).toBe("বাংলা · Bengali");
    expect(LANGUAGES.find((l) => l.code === "ar")?.dir).toBe("rtl");
  });

  it("fills {names}, falls back to English, and picks the saved or browser language", () => {
    const t = translator({ Save: "Speichern", "{n} left": "{n} übrig" });
    expect(t("Save")).toBe("Speichern");
    expect(t("Cancel")).toBe("Cancel");
    expect(t("{n} left", { n: 2 })).toBe("2 übrig");
    expect(fill("{a} {b}", { a: 1 })).toBe("1 {b}");
    expect(pickLanguage("de", ["fr"]).code).toBe("de");
    expect(pickLanguage(null, ["fr-CA"]).code).toBe("fr");
    expect(pickLanguage(null, ["xx"]).code).toBe("en");
  });

  const all = texts();
  it("finds the popup's texts", () => {
    for (const text of ["Download with Mohona", "Hide on {site}", "Start Mohona", "Total speed", "Mohona {version} is running", "Waiting", "Desert"]) expect(all).toContain(text);
  });

  it.each(files)("%s has every popup text and the same {names} as English", (file) => {
    const dict = JSON.parse(readFileSync(join(locales, file), "utf8")) as Record<string, string>;
    const missing = all.filter((t) => !dict[t]);
    const broken = all.filter((t) => dict[t] && placeholders(dict[t]) !== placeholders(t));
    expect({ missing, broken }).toEqual({ missing: [], broken: [] });
    expect(dict._name && dict._english).toBeTruthy();
  });
});
