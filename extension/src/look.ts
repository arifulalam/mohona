// How the button on videos looks and speaks: the language and colour theme the
// person chose in the popup. The popup keeps its choice in the browser's
// storage; the background script turns it into the few words and colours the
// page's button needs, so the page script stays small.

import { LANGUAGES, pickLanguage, translator } from "./i18n";
import { NAMED_THEMES } from "./themes.generated";

export interface Colors {
  dark: boolean;
  panel: string;
  text: string;
  muted: string;
  line: string;
  accent: string;
  accentText: string;
  ok: string;
  bad: string;
  hero: string;
  heroText: string;
}

export interface Look {
  lang: string;
  dir: "ltr" | "rtl";
  /** English text to translation, only for the texts the button uses. */
  words: Record<string, string>;
  /** null: follow the system's light or dark. */
  colors: Colors | null;
}

/** Every text of the button (the others are the popup's). */
export const BUTTON_TEXTS = [
  "Download with Mohona",
  "Looking for downloads…",
  "Mohona is not available. Is it installed and running?",
  "YouTube sends video and sound in many small signed pieces, so there is no single file to take yet. Support for it is planned.",
  "Nothing to download was found yet. Press play, wait a moment, and try again.",
  "Download",
  "Sending…",
  "Sent",
  "Failed",
  "Hide on {site}",
  "Close",
  "file",
];

const LIGHT: Colors = {
  dark: false, panel: "#ffffff", text: "#1b1f27", muted: "#5a6370", line: "#e2e5ea", accent: "#1259b0", accentText: "#ffffff",
  ok: "#1b7f3b", bad: "#b3261e", hero: "linear-gradient(135deg,#2b78d4,#1259b0)", heroText: "#ffffff",
};
const DARK: Colors = {
  dark: true, panel: "#1c2027", text: "#e8eaee", muted: "#9aa3b0", line: "#2d333d", accent: "#5b9df0", accentText: "#08121f",
  ok: "#5cc27a", bad: "#ff8a80", hero: "linear-gradient(135deg,#2a5fa8,#14325c)", heroText: "#ffffff",
};

export function colorsFor(theme: string | null): Colors | null {
  if (theme === "light") return LIGHT;
  if (theme === "dark") return DARK;
  const named = NAMED_THEMES.find((t) => t.id === theme);
  return named ? { ...named } : null;
}

export function lookFor(theme: string | null, language: string | null, browser: readonly string[]): Look {
  const lang = pickLanguage(language, browser, LANGUAGES);
  const t = translator(lang.dict);
  const words: Record<string, string> = {};
  for (const text of BUTTON_TEXTS) {
    const translated = t(text);
    if (translated !== text) words[text] = translated;
  }
  return { lang: lang.code, dir: lang.dir, words, colors: colorsFor(theme) };
}
