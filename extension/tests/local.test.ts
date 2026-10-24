import { describe, expect, it } from "vitest";
import { bytesToBase64, isLocalUrl, MAX_SIZE, PIECE, saveLocal, sliceInPage, sourceFromBlob, sourceFromData, type Source } from "../src/local";
import { localTarget } from "../src/menu";
import { NativeClient, type Port } from "../src/native";

/** A fake host that remembers what was saved. */
function host(opts: { notRunning?: boolean; failPiece?: number } = {}) {
  const log: any[] = [];
  const pieces: string[] = [];
  let running = !opts.notRunning;
  const listeners: ((m: unknown) => void)[] = [];
  const port: Port = {
    onMessage: { addListener: (l) => void listeners.push(l) },
    onDisconnect: { addListener: () => undefined },
    disconnect: () => undefined,
    postMessage(m: any) {
      log.push(m.type);
      let answer: any;
      switch (m.type) {
        case "startEngine":
          running = true;
          answer = { type: "engine", running: false, started: true };
          break;
        case "saveStart":
          answer = running ? { type: "saving", transfer: 9 } : { type: "error", code: "engine_not_running", message: "no" };
          break;
        case "savePiece":
          if (m.index === opts.failPiece) answer = { type: "error", code: "engine", message: "disk full" };
          else {
            pieces[m.index] = m.data;
            answer = { type: "piece", index: m.index };
          }
          break;
        case "saveEnd":
          answer = { type: "saved", path: "/dl/scan.png", name: "scan.png" };
          break;
        default:
          answer = { type: "aborted" };
      }
      queueMicrotask(() => listeners.forEach((l) => l({ ...answer, requestId: m.requestId })));
    },
  };
  return { client: new NativeClient(() => port), log, pieces };
}

const source = (size: number, mime = "image/png"): Source => ({
  size,
  mime,
  slice: async (start, end) => `${start}-${end}`,
});

describe("saveLocal", () => {
  it("sends the file in pieces and returns where it was saved", async () => {
    const h = host();
    const result = await saveLocal(h.client, source(PIECE * 2 + 10), "scan");
    expect(result).toEqual({ kind: "saved", path: "/dl/scan.png", name: "scan.png" });
    expect(h.log).toEqual(["saveStart", "savePiece", "savePiece", "savePiece", "saveEnd"]);
    expect(h.pieces).toEqual([`0-${PIECE}`, `${PIECE}-${PIECE * 2}`, `${PIECE * 2}-${PIECE * 2 + 10}`]);
  });

  it("starts the engine when it is not running", async () => {
    const h = host({ notRunning: true });
    expect((await saveLocal(h.client, source(10), "x")).kind).toBe("saved");
    expect(h.log.slice(0, 3)).toEqual(["saveStart", "startEngine", "saveStart"]);
  });

  it("aborts the transfer when a piece fails", async () => {
    const h = host({ failPiece: 1 });
    const result = await saveLocal(h.client, source(PIECE + 1), "x");
    expect(result).toEqual({ kind: "failed", message: "disk full" });
    expect(h.log.at(-1)).toBe("saveAbort");
  });

  it("refuses empty and huge files before asking the engine", async () => {
    const h = host();
    expect((await saveLocal(h.client, source(0), "x")).kind).toBe("failed");
    expect((await saveLocal(h.client, source(MAX_SIZE + 1), "x")).kind).toBe("failed");
    expect(h.log).toEqual([]);
  });

  it("reports a source that cannot be read, and aborts", async () => {
    const h = host();
    const broken: Source = { size: 10, slice: async () => { throw new Error("released"); } };
    expect(await saveLocal(h.client, broken, "x")).toEqual({ kind: "failed", message: "released" });
    expect(h.log.at(-1)).toBe("saveAbort");
  });
});

describe("sources", () => {
  it("reads a data: address into base64 pieces", async () => {
    const url = "data:text/plain;base64," + btoa("hello world");
    const s = await sourceFromData(url);
    expect(s.size).toBe(11);
    expect(s.mime).toBe("text/plain");
    expect(await s.slice(0, 5)).toBe(btoa("hello"));
  });

  it("reads a blob: through the page, and says when the page no longer has it", async () => {
    const calls: number[][] = [];
    const s = await sourceFromBlob("blob:x", async (a, b) => {
      calls.push([a, b]);
      return { size: 3 * PIECE, type: "image/png", data: `d${a}` };
    });
    expect(s).toMatchObject({ size: 3 * PIECE, mime: "image/png" });
    expect(await s.slice(0, PIECE)).toBe("d0");
    expect(await s.slice(PIECE, 2 * PIECE)).toBe(`d${PIECE}`);
    expect(calls).toEqual([[0, PIECE], [PIECE, 2 * PIECE]]); // the first piece is not read twice
    await expect(sourceFromBlob("blob:gone", async () => undefined)).rejects.toThrow(/no longer/);
  });

  it("the in-page function reads a real blob slice", async () => {
    const blob = new Blob([new Uint8Array([1, 2, 3, 4, 5])], { type: "application/octet-stream" });
    const url = "blob:test";
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async () => new Response(blob)) as typeof fetch;
    try {
      expect(await sliceInPage(url, 1, 3)).toEqual({ size: 5, type: "application/octet-stream", data: btoa("\x02\x03") });
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  it("encodes bytes like btoa and recognises local addresses", () => {
    expect(bytesToBase64(new Uint8Array([104, 105]))).toBe(btoa("hi"));
    expect(bytesToBase64(new Uint8Array(100_000).fill(255))).toBe(btoa("\xff".repeat(100_000)));
    expect(isLocalUrl("data:text/plain,hi")).toBe(true);
    expect(isLocalUrl("BLOB:https://a/1")).toBe(true);
    expect(isLocalUrl("https://a/")).toBe(false);
    expect(isLocalUrl(undefined)).toBe(false);
  });

  it("menu clicks on data: and blob: addresses are found", () => {
    expect(localTarget({ srcUrl: "data:image/png;base64,AAAA" })).toBe("data:image/png;base64,AAAA");
    expect(localTarget({ linkUrl: "blob:https://a/1" })).toBe("blob:https://a/1");
    expect(localTarget({ linkUrl: "https://a/f.zip", srcUrl: "data:x" })).toBeNull();
    expect(localTarget({})).toBeNull();
  });
});
