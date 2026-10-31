import { tr } from "./i18n";
import { NAMED_THEMES } from "./themes.generated";

export type Theme = string;

export const THEMES: { id: Theme; label: string }[] = [
  { id: "system", label: tr("System") },
  { id: "light", label: tr("Light") },
  { id: "dark", label: tr("Dark") },
  ...NAMED_THEMES.map((t) => ({ id: t.id, label: t.label })),
];

const KEY = "mohona.theme";

export function loadTheme(): Theme {
  try {
    const v = localStorage.getItem(KEY);
    if (v && THEMES.some((t) => t.id === v)) return v;
  } catch {
    /* storage may be unavailable */
  }
  return "system";
}

export function saveTheme(theme: Theme): void {
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    /* ignore */
  }
}

/** "system" follows the operating system; the others are fixed. */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}
