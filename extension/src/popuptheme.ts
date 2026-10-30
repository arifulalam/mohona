import { NAMED_THEMES } from "./themes.generated";

export const BASE_THEMES = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
] as const;

const KEY = "mohona.theme";

/** Every theme the person can choose, in the order shown. */
export const ALL_THEMES: { id: string; label: string }[] = [...BASE_THEMES, ...NAMED_THEMES.map((t) => ({ id: t.id, label: t.label }))];

export function loadTheme(storage: Pick<Storage, "getItem"> | null): string {
  try {
    const value = storage?.getItem(KEY);
    if (value && ALL_THEMES.some((t) => t.id === value)) return value;
  } catch {
    /* storage may be unavailable */
  }
  return "system";
}

export function saveTheme(storage: Pick<Storage, "setItem"> | null, theme: string): void {
  try {
    storage?.setItem(KEY, theme);
  } catch {
    /* ignore */
  }
}

export function applyTheme(root: HTMLElement, theme: string): void {
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}
