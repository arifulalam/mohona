import { describe, expect, it } from "vitest";
import { addDownload, NativeClient, startEngine, status, type Port } from "../src/native";

/** A fake host: `reply` decides what each question gets (or nothing). */
function fake(reply: (question: any) => unknown | undefined) {
  const sent: any[] = [];
  let connects = 0;
  const ports: FakePort[] = [];
  class FakePort implements Port {
    messageListeners: ((m: unknown) => void)[] = [];
    disconnectListeners: (() => void)[] = [];
    onMessage = { addListener: (l: (m: unknown) => void) => void this.messageListeners.push(l) };
    onDisconnect = { addListener: (l: () => void) => void this.disconnectListeners.push(l) };
    postMessage(message: any) {
      sent.push(message);
      const answer = reply(message) as Record<string, unknown> | undefined;
      if (answer) {
        queueMicrotask(() =>
          this.messageListeners.forEach((l) => l({ ...answer, requestId: message.requestId })),
        );
      }
    }
    disconnect() {
      this.drop();
    }
    drop() {
      this.disconnectListeners.forEach((l) => l());
    }
  }
  const client = new NativeClient(() => {
    connects++;
    const port = new FakePort();
    ports.push(port);
    return port;
  });
  return { client, sent, ports, connects: () => connects };
}

describe("NativeClient", () => {
  it("matches answers to questions by requestId", async () => {
    const f = fake((q) => ({ type: "added", id: q.requestId * 10 }));
    const [a, b] = await Promise.all([
      f.client.request({ type: "add", url: "https://a/x" }),
      f.client.request({ type: "add", url: "https://b/y" }),
    ]);
    expect(a).toMatchObject({ id: 10 });
    expect(b).toMatchObject({ id: 20 });
    expect(f.connects()).toBe(1);
  });

  it("times out when nothing answers", async () => {
    const f = fake(() => undefined);
    await expect(f.client.request({ type: "hello" }, 20)).rejects.toThrow(/in time/);
  });

  it("fails waiting questions when the host goes away and reconnects after", async () => {
    const f = fake((q) => (q.type === "hello" ? { type: "hello" } : undefined));
    const waiting = f.client.request({ type: "startEngine" }, 1000);
    f.ports[0]!.drop();
    await expect(waiting).rejects.toThrow(/not available/);
    await f.client.request({ type: "hello" });
    expect(f.connects()).toBe(2);
  });

  it("reports a failure to connect", async () => {
    const client = new NativeClient(() => {
      throw new Error("Specified native messaging host not found.");
    });
    await expect(client.request({ type: "hello" })).rejects.toThrow(/not found/);
  });
});

describe("addDownload", () => {
  it("adds", async () => {
    const f = fake(() => ({ type: "added", id: 5 }));
    expect(await addDownload(f.client, { url: "https://a/x" })).toEqual({ kind: "added", id: 5 });
  });

  it("starts the engine once and tries again", async () => {
    let running = false;
    const f = fake((q) => {
      if (q.type === "startEngine") {
        running = true;
        return { type: "engine", running: false, started: true };
      }
      return running
        ? { type: "added", id: 1 }
        : { type: "error", code: "engine_not_running", message: "no" };
    });
    expect(await addDownload(f.client, { url: "https://a/x" })).toEqual({ kind: "added", id: 1 });
    expect(f.sent.map((m) => m.type)).toEqual(["add", "startEngine", "add"]);
  });

  it("gives up when the engine cannot be started", async () => {
    const f = fake((q) =>
      q.type === "startEngine"
        ? { type: "error", code: "no_engine", message: "not found" }
        : { type: "error", code: "engine_not_running", message: "no" },
    );
    expect(await addDownload(f.client, { url: "https://a/x" })).toEqual({
      kind: "error",
      code: "no_engine",
      message: "not found",
    });
    expect(f.sent).toHaveLength(2);
  });

  it("passes duplicates and errors on, and survives a missing host", async () => {
    expect(
      await addDownload(fake(() => ({ type: "duplicate", message: "already" })).client, { url: "u" }),
    ).toEqual({ kind: "duplicate", message: "already", exists: false });
    expect(
      await addDownload(
        fake(() => ({ type: "duplicate", message: "already", id: 7, status: "completed", exists: true })).client,
        { url: "u" },
      ),
    ).toEqual({ kind: "duplicate", message: "already", id: 7, status: "completed", exists: true });
    expect(
      await addDownload(fake(() => ({ type: "error", code: "bad_url", message: "x" })).client, { url: "u" }),
    ).toEqual({ kind: "error", code: "bad_url", message: "x" });
    const gone = new NativeClient(() => {
      throw new Error("host not found");
    });
    expect(await addDownload(gone, { url: "u" })).toMatchObject({
      kind: "error",
      code: "host_unavailable",
    });
  });

  it("sends the browser's context along", async () => {
    const f = fake(() => ({ type: "added", id: 1 }));
    await addDownload(f.client, { url: "https://a/x", cookies: "a=b", referrer: "https://a/" });
    expect(f.sent[0]).toMatchObject({ type: "add", url: "https://a/x", cookies: "a=b", referrer: "https://a/" });
  });
});

describe("status", () => {
  const hello = (extra: object) => ({ type: "hello", protocol: 1, version: "0.0.1", engineVersion: null, ...extra });
  it("tells ready, stopped, incompatible and missing apart", async () => {
    expect(await status(fake(() => hello({ engineRunning: true })).client)).toEqual({ state: "ready", version: "0.0.1" });
    expect(await status(fake(() => hello({ engineRunning: false })).client)).toEqual({ state: "engine_stopped", version: "0.0.1" });
    expect(await status(fake(() => hello({ engineRunning: true, protocol: 2 })).client)).toMatchObject({ state: "incompatible" });
    const gone = new NativeClient(() => {
      throw new Error("nope");
    });
    expect(await status(gone)).toEqual({ state: "host_missing", message: "nope" });
  });
});

describe("startEngine", () => {
  it("is null on success and an error otherwise", async () => {
    expect(await startEngine(fake(() => ({ type: "engine", running: false, started: true })).client)).toBeNull();
    expect(
      await startEngine(fake(() => ({ type: "error", code: "no_engine", message: "x" })).client),
    ).toMatchObject({ kind: "error", code: "no_engine" });
  });
});
