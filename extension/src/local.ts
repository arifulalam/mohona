// Files that only the browser has: `data:` addresses hold their bytes, `blob:`
// addresses point into the page that made them. Mohona's engine cannot fetch
// either, so the extension reads the bytes and sends them over in pieces.

import type { NativeClient } from "./native";
import { startEngine } from "./native";

/** Each piece, before base64; the host and engine accept up to 768 KiB. */
export const PIECE = 512 * 1024;
/** More than this is refused here already (the engine has the same limit). */
export const MAX_SIZE = 1024 * 1024 * 1024;

export type SaveResult =
  | { kind: "saved"; path: string; name: string }
  | { kind: "failed"; message: string };

/** Where the bytes come from. `slice` returns base64. */
export interface Source {
  size: number;
  mime?: string;
  slice(start: number, end: number): Promise<string>;
}

export function isLocalUrl(url: string | undefined): url is string {
  return !!url && /^(data|blob):/i.test(url);
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/** A `data:` address, read with the browser's own decoder. */
export async function sourceFromData(url: string, read: typeof fetch = fetch): Promise<Source> {
  const blob = await (await read(url)).blob();
  return {
    size: blob.size,
    mime: blob.type || undefined,
    slice: async (start, end) => bytesToBase64(new Uint8Array(await blob.slice(start, end).arrayBuffer())),
  };
}

/** The function that runs inside the page for a `blob:` address (it must not use anything outside itself). */
export async function sliceInPage(
  url: string,
  start: number,
  end: number,
): Promise<{ size: number; type: string; data: string }> {
  const blob = await (await fetch(url)).blob();
  const bytes = new Uint8Array(await blob.slice(start, end).arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return { size: blob.size, type: blob.type, data: btoa(binary) };
}

/** A `blob:` address, read inside the page that owns it. */
export async function sourceFromBlob(
  url: string,
  run: (start: number, end: number) => Promise<{ size: number; type: string; data: string } | undefined>,
): Promise<Source> {
  const first = await run(0, PIECE);
  if (!first) throw new Error("the page no longer has this file (it may have been released)");
  const cache = new Map<number, string>([[0, first.data]]);
  return {
    size: first.size,
    mime: first.type || undefined,
    slice: async (start, end) => {
      const known = start === 0 && end === PIECE ? cache.get(0) : undefined;
      if (known !== undefined) return known;
      const part = await run(start, end);
      if (!part) throw new Error("the page no longer has this file");
      return part.data;
    },
  };
}

/** Sends a file to Mohona in pieces; the engine decides the final name and type. */
export async function saveLocal(client: NativeClient, source: Source, name: string): Promise<SaveResult> {
  if (source.size <= 0) return { kind: "failed", message: "the file is empty" };
  if (source.size > MAX_SIZE) return { kind: "failed", message: "the file is larger than 1 GiB" };
  let transfer: number | undefined;
  try {
    const begin = { type: "saveStart", name, mime: source.mime } as const;
    let answer = await client.request(begin);
    if (answer.type === "error" && answer.code === "engine_not_running") {
      const started = await startEngine(client);
      if (started) return { kind: "failed", message: started.kind === "error" ? started.message : "Mohona did not start" };
      answer = await client.request(begin);
    }
    if (answer.type !== "saving") {
      return { kind: "failed", message: answer.type === "error" ? answer.message : "Mohona gave an unexpected answer" };
    }
    transfer = answer.transfer;
    for (let index = 0, start = 0; start < source.size; index++, start += PIECE) {
      const data = await source.slice(start, Math.min(start + PIECE, source.size));
      const sent = await client.request({ type: "savePiece", transfer, index, data }, 30_000);
      if (sent.type !== "piece") {
        return { kind: "failed", message: sent.type === "error" ? sent.message : "Mohona gave an unexpected answer" };
      }
    }
    const done = await client.request({ type: "saveEnd", transfer }, 30_000);
    transfer = undefined;
    return done.type === "saved"
      ? { kind: "saved", path: done.path, name: done.name }
      : { kind: "failed", message: done.type === "error" ? done.message : "Mohona gave an unexpected answer" };
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  } finally {
    if (transfer !== undefined) {
      await client.request({ type: "saveAbort", transfer }, 5_000).catch(() => undefined);
    }
  }
}
