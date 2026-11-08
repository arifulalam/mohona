const KEY = "mohona.droptarget";

/** Is the small drop target on the screen wanted? (Remembered on this computer.) */
export function loadDropTarget(): boolean {
  try {
    return localStorage.getItem(KEY) === "on";
  } catch {
    return false;
  }
}

export function saveDropTarget(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* ignore */
  }
}

/** Every link in dropped text, one per line: http, https, ftp and magnet links, each once. */
export function linksIn(text: string | null | undefined, one: (t: string) => string | null): string[] {
  const found: string[] = [];
  for (const raw of (text ?? "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const link = line.startsWith("magnet:?") ? line : one(line);
    if (link && !found.includes(link)) found.push(link);
  }
  return found;
}
