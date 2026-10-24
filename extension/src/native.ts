import type { AddDetails, Answer, Outcome, Question } from "./protocol";
import { PROTOCOL } from "./protocol";

/** The part of a browser's native messaging port that is used. */
export interface Port {
  postMessage(message: unknown): void;
  onMessage: { addListener(listener: (message: unknown) => void): void };
  onDisconnect: { addListener(listener: () => void): void };
  disconnect(): void;
}

interface Pending {
  resolve(answer: Answer): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * One long-lived connection to the host. Questions carry a `requestId` and
 * are matched with the answer that repeats it. A lost connection fails the
 * questions waiting on it; the next question connects again.
 */
export class NativeClient {
  private port: Port | undefined;
  private next = 1;
  private pending = new Map<number, Pending>();

  constructor(private readonly connect: () => Port) {}

  request(question: Question, timeoutMs = 15_000): Promise<Answer> {
    return new Promise((resolve, reject) => {
      let port: Port;
      try {
        port = this.port ?? this.open();
      } catch (e) {
        reject(asError(e));
        return;
      }
      const id = this.next++;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error("Mohona did not answer in time"));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        port.postMessage({ ...question, requestId: id });
      } catch (e) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(asError(e));
      }
    });
  }

  close(): void {
    this.port?.disconnect();
    this.lost(new Error("closed"));
  }

  private open(): Port {
    const port = this.connect();
    port.onMessage.addListener((message) => this.answer(message));
    port.onDisconnect.addListener(() => {
      if (this.port === port) this.lost(new Error("the Mohona host is not available"));
    });
    this.port = port;
    return port;
  }

  private answer(message: unknown): void {
    const answer = message as Answer & { requestId?: number };
    const entry =
      typeof answer.requestId === "number" ? this.pending.get(answer.requestId) : undefined;
    if (!entry || typeof answer.requestId !== "number") return;
    clearTimeout(entry.timer);
    this.pending.delete(answer.requestId);
    entry.resolve(answer);
  }

  private lost(error: Error): void {
    this.port = undefined;
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.pending.clear();
  }
}

function asError(e: unknown): Error {
  return e instanceof Error ? e : new Error(String(e));
}

function failure(code: string, message: string): Outcome {
  return { kind: "error", code, message };
}

/**
 * Hands a download to Mohona. When the engine is not running it is started
 * once and the add is tried again.
 */
export async function addDownload(client: NativeClient, details: AddDetails): Promise<Outcome> {
  try {
    let answer = await client.request({ type: "add", ...details });
    if (answer.type === "error" && answer.code === "engine_not_running") {
      const started = await client.request({ type: "startEngine" }, 30_000);
      if (started.type === "error") return failure(started.code, started.message);
      answer = await client.request({ type: "add", ...details });
    }
    switch (answer.type) {
      case "added":
        return { kind: "added", id: answer.id };
      case "duplicate":
        return { kind: "duplicate", message: answer.message };
      case "error":
        return failure(answer.code, answer.message);
      default:
        return failure("unexpected", "Mohona gave an unexpected answer");
    }
  } catch (e) {
    return failure("host_unavailable", asError(e).message);
  }
}

export type Status =
  | { state: "ready"; version: string }
  | { state: "engine_stopped"; version: string }
  | { state: "host_missing"; message: string }
  | { state: "incompatible"; message: string };

/** Is the host there, and is the engine running? */
export async function status(client: NativeClient): Promise<Status> {
  try {
    const answer = await client.request({ type: "hello" }, 5_000);
    if (answer.type !== "hello") {
      return { state: "host_missing", message: "The host answered something unexpected" };
    }
    if (answer.protocol !== PROTOCOL) {
      return {
        state: "incompatible",
        message: `The host speaks protocol ${answer.protocol}, this extension ${PROTOCOL}. Update Mohona or the extension.`,
      };
    }
    return answer.engineRunning
      ? { state: "ready", version: answer.version }
      : { state: "engine_stopped", version: answer.version };
  } catch (e) {
    return { state: "host_missing", message: asError(e).message };
  }
}

/** Starts the engine and reports whether it came up. */
export async function startEngine(client: NativeClient): Promise<Outcome | null> {
  try {
    const answer = await client.request({ type: "startEngine" }, 30_000);
    return answer.type === "error" ? failure(answer.code, answer.message) : null;
  } catch (e) {
    return failure("host_unavailable", asError(e).message);
  }
}

/**
 * Hands an HLS or DASH stream to Mohona, which runs `mohona stream` for it.
 * Starts the engine once when it is not running, like `addDownload`.
 */
export async function startStream(client: NativeClient, details: AddDetails): Promise<Outcome> {
  try {
    let answer = await client.request({ type: "stream", ...details }, 15_000);
    if (answer.type === "error" && answer.code === "engine_not_running") {
      const started = await client.request({ type: "startEngine" }, 30_000);
      if (started.type === "error") return failure(started.code, started.message);
      answer = await client.request({ type: "stream", ...details }, 15_000);
    }
    switch (answer.type) {
      case "streamStarted":
        return { kind: "streaming", job: answer.job, folder: answer.folder };
      case "error":
        return failure(answer.code, answer.message);
      default:
        return failure("unexpected", "Mohona gave an unexpected answer");
    }
  } catch (e) {
    return failure("host_unavailable", asError(e).message);
  }
}
