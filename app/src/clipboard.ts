import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, Row } from "./types";

const KEY = "mohona.clipboard";

export function loadClipboardWatch(): boolean {
  try {
    return localStorage.getItem(KEY) === "on";
  } catch {
    return false;
  }
}

export function saveClipboardWatch(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* ignore */
  }
}

/** The address when the whole text is one http, https or ftp link; otherwise null. */
export function linkIn(text: string | null): string | null {
  const t = text?.trim();
  if (!t || t.length > 2048 || /\s/.test(t)) return null;
  try {
    const url = new URL(t);
    return ["http:", "https:", "ftp:"].includes(url.protocol) && url.hostname.includes(".") ? t : null;
  } catch {
    return null;
  }
}

/**
 * Looks at the clipboard when the window comes to the front (and once at the
 * start) and offers a copied link, once. A link that is already in the list,
 * or that was offered or dismissed before, is not offered again.
 */
export function useClipboardOffer(api: Api, on: boolean, rows: Row[], running: boolean) {
  const [offer, setOffer] = useState<string | null>(null);
  const handled = useRef(new Set<string>());
  const known = useRef(rows);
  known.current = rows;

  const look = useCallback(async () => {
    if (!on || !running) return;
    const link = linkIn(await api.clipboard().catch(() => null));
    if (!link || handled.current.has(link) || known.current.some((r) => r.url === link)) return;
    handled.current.add(link);
    setOffer(link);
  }, [api, on, running]);

  useEffect(() => {
    void look();
    window.addEventListener("focus", look);
    return () => window.removeEventListener("focus", look);
  }, [look]);

  useEffect(() => {
    if (!on) setOffer(null);
  }, [on]);

  return { offer, dismiss: () => setOffer(null) };
}
