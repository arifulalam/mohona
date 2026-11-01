/** One language file: English text (the key) to the translation. Keys starting with "_" describe the file. */
export type Dict = Record<string, string>;

export interface Language {
  code: string;
  /** The language's own name, as its speakers write it. */
  name: string;
  /** The same name in English, shown beside it. */
  english: string;
  dir: "ltr" | "rtl";
  dict: Dict;
}

export type Vars = Record<string, string | number>;
export type Translate = (text: string, vars?: Vars) => string;

// Every file in locales/ is a language: drop in `xx.json` and it appears in the list.
const files = import.meta.glob("./locales/*.json", { eager: true, import: "default" }) as Record<string, Dict>;

export const ENGLISH: Language = { code: "en", name: "English", english: "English", dir: "ltr", dict: {} };

export function languageFrom(code: string, dict: Dict): Language {
  return { code, name: dict._name || code, english: dict._english || "", dir: dict._dir === "rtl" ? "rtl" : "ltr", dict };
}

export const LANGUAGES: Language[] = [
  ENGLISH,
  ...Object.entries(files)
    .map(([path, dict]) => languageFrom(path.replace(/^.*\/(.+)\.json$/, "$1"), dict))
    .filter((l) => l.code !== "en" && !l.code.startsWith("_"))
    .sort((a, b) => a.name.localeCompare(b.name)),
];

/** "বাংলা · Bengali"; just "English" when both are the same. */
export const label = (l: Language): string => (l.english && l.english !== l.name ? `${l.name} · ${l.english}` : l.name);

/** Replaces {name} in a text; an unknown {name} stays as it is. */
export function fill(text: string, vars?: Vars): string {
  return vars ? text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole)) : text;
}

export function translator(dict: Dict): Translate {
  return (text, vars) => fill(dict[text] || text, vars);
}

/** The saved language, else the browser's, else English. */
export function pickLanguage(saved: string | null, browser: readonly string[], list: Language[] = LANGUAGES): Language {
  const find = (code: string) => list.find((l) => l.code.toLowerCase() === code.toLowerCase());
  if (saved) {
    const l = find(saved);
    if (l) return l;
  }
  for (const tag of browser) {
    const l = find(tag) ?? find(tag.split("-")[0] ?? "");
    if (l) return l;
  }
  return ENGLISH;
}

const KEY = "mohona.language";

export function loadLanguage(storage: Pick<Storage, "getItem"> | null, browser: readonly string[]): Language {
  let saved: string | null = null;
  try {
    saved = storage?.getItem(KEY) ?? null;
  } catch {
    /* storage may be unavailable */
  }
  return pickLanguage(saved, browser);
}

export function saveLanguage(storage: Pick<Storage, "setItem"> | null, code: string): void {
  try {
    storage?.setItem(KEY, code);
  } catch {
    /* ignore */
  }
}
