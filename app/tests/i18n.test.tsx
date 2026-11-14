import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { findTexts, review } from "../scripts/i18n.mjs";
import { App } from "../src/App";
import { fill, label, LANGUAGES, languageFrom, pickLanguage, translator } from "../src/i18n";
import type { Api } from "../src/types";

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.documentElement.removeAttribute("dir");
});

const locales = join(process.cwd(), "src", "locales");
const files = readdirSync(locales).filter((n) => /^[a-z].*\.json$/.test(n));

describe("translating", () => {
  it("fills {names}, leaves unknown ones, and falls back to English", () => {
    expect(fill("{a} of {b}", { a: 1, b: "x" })).toBe("1 of x");
    expect(fill("{a} {zzz}", { a: 1 })).toBe("1 {zzz}");
    const t = translator({ Save: "Speichern", "{n} left": "{n} übrig" });
    expect(t("Save")).toBe("Speichern");
    expect(t("Cancel")).toBe("Cancel");
    expect(t("{n} left", { n: 3 })).toBe("3 übrig");
    expect(translator({ Save: "" })("Save")).toBe("Save");
  });

  it("picks the saved language, then the browser's, then English", () => {
    const list = [languageFrom("en", {}), languageFrom("pt", { _name: "Português" }), languageFrom("pt-BR", { _name: "Português (Brasil)" }), languageFrom("bn", { _name: "বাংলা" })];
    expect(pickLanguage("bn", ["fr"], list).code).toBe("bn");
    expect(pickLanguage(null, ["fr-FR", "bn-BD"], list).code).toBe("bn");
    expect(pickLanguage(null, ["pt-BR"], list).code).toBe("pt-BR");
    expect(pickLanguage(null, ["pt-PT"], list).code).toBe("pt");
    expect(pickLanguage("xx", ["zz"], list).code).toBe("en");
    expect(pickLanguage(null, [], list).code).toBe("en");
  });

  it("offers English first and every file in locales/ with its own name", () => {
    expect(LANGUAGES[0]?.code).toBe("en");
    expect(LANGUAGES.map((l) => l.code).sort()).toEqual(["en", ...files.map((f) => f.replace(".json", ""))].sort());
    for (const l of LANGUAGES) expect(l.name.length).toBeGreaterThan(0);
    expect(label(LANGUAGES.find((l) => l.code === "bn")!)).toBe("বাংলা · Bengali");
    expect(label(LANGUAGES[0]!)).toBe("English");
    for (const code of ["bn", "hi", "ur", "ar", "es", "fr", "de", "pt", "ru", "zh", "ja", "id", "tr"]) expect(LANGUAGES.some((l) => l.code === code)).toBe(true);
    expect(LANGUAGES.find((l) => l.code === "ar")?.dir).toBe("rtl");
    expect(LANGUAGES.find((l) => l.code === "ur")?.dir).toBe("rtl");
    expect(LANGUAGES.find((l) => l.code === "bn")?.dir).toBe("ltr");
  });

  const texts = findTexts();
  it.each(files)("%s has every text, no leftovers and the same {names} as the English", (file) => {
    const dict = JSON.parse(readFileSync(join(locales, file), "utf8")) as Record<string, string>;
    const { missing, stale, broken } = review(dict, texts);
    // A language someone is still working on may lack texts (they show in English); the ones that ship may not.
    const shipped = ["ar", "bn", "de", "es", "fr", "hi", "id", "ja", "pt", "ru", "tr", "ur", "zh"].includes(file.replace(".json", ""));
    expect({ missing: shipped ? missing : [], stale, broken }).toEqual({ missing: [], stale: [], broken: [] });
    expect(dict._name).toBeTruthy();
    expect(dict._english).toBeTruthy();
    expect(["ltr", "rtl"]).toContain(dict._dir);
  });

  it("finds the texts of the window, including those in tables", () => {
    for (const text of ["Settings", "Waiting", "Newest first", "Desert", "Pause {name}", "Mon"]) expect(texts).toContain(text);
  });
});

describe("the window in another language", () => {
  const api = {
    status: async () => ({ running: true, version: "1", message: null }),
    list: async () => [],
    plan: async () => ({ windows: [], openNow: true, action: null, countdown: null, message: null }),
    categories: async () => [],
    queues: async () => [],
    clipboard: async () => null,
    update: async () => ({ repo: "", current: "0.1.0", latest: null, newer: false, url: null, notes: null, installable: false }),
  } as unknown as Api;

  it("switches the words, remembers the choice and sets the direction", async () => {
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(await screen.findByLabelText("Language"), "bn");
    expect(await screen.findByRole("button", { name: "সেটিংস" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "সময়সূচি" })).toBeTruthy();
    expect(localStorage.getItem("mohona.language")).toBe("bn");
    expect(document.documentElement.lang).toBe("bn");
    expect(document.documentElement.dir).toBe("ltr");
    await userEvent.selectOptions(screen.getByLabelText("ভাষা"), "ar");
    expect(await screen.findByRole("button", { name: "الإعدادات" })).toBeTruthy();
    expect(document.documentElement.dir).toBe("rtl");
    await userEvent.selectOptions(screen.getByLabelText("اللغة"), "English");
    expect(await screen.findByRole("button", { name: "Settings" })).toBeTruthy();
    expect(document.documentElement.dir).toBe("ltr");
  });

  it("starts in the saved language", async () => {
    localStorage.setItem("mohona.language", "de");
    render(<App api={api} every={50} />);
    expect(await screen.findByRole("button", { name: "Einstellungen" })).toBeTruthy();
  });
});
