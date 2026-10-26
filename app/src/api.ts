import type { Api, EngineStatus, Row } from "./types";

/** True inside the Tauri window; false in a plain browser (development, tests). */
export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function tauriApi(): Api {
  const call = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      return await invoke<T>(command, args);
    } catch (e) {
      throw new Error(typeof e === "string" ? e : String((e as Error)?.message ?? e));
    }
  };
  return {
    status: () => call<EngineStatus>("engine_status"),
    start: () => call<EngineStatus>("start_engine"),
    list: () => call<Row[]>("list_downloads"),
    add: (url, folder) => call<number>("add_download", { url, folder }),
    pause: (id) => call<void>("pause_download", { id }),
    resume: (id) => call<void>("resume_download", { id }),
    cancel: (id) => call<void>("cancel_download", { id }),
    remove: (id, deleteFile) => call<void>("remove_download", { id, deleteFile }),
  };
}

/** A pretend engine, so the window can be tried in a browser. */
export function demoApi(tick = 1000): Api {
  let running = false;
  let next = 4;
  let rows: Row[] = [
    demoRow(1, "ubuntu-24.04-desktop-amd64.iso", "downloading", 3_100_000_000, 6_200_000_000, 8_400_000),
    demoRow(2, "lecture-07.mp4", "paused", 120_000_000, 480_000_000, 0),
    demoRow(3, "dataset.zip", "completed", 52_000_000, 52_000_000, 0),
  ];
  setInterval(() => {
    if (!running) return;
    rows = rows.map((r) => {
      if (r.status !== "downloading" || r.size === null) return r;
      const downloaded = Math.min(r.size, r.downloaded + r.speed * (tick / 1000));
      return downloaded >= r.size
        ? { ...r, downloaded, percent: 100, speed: 0, eta: null, status: "completed" }
        : { ...r, downloaded, percent: (downloaded / r.size) * 100, eta: Math.ceil((r.size - downloaded) / r.speed) };
    });
  }, tick);
  const status = (): EngineStatus => ({ running, version: running ? "demo" : null, message: null });
  const need = () => {
    if (!running) throw new Error("Mohona is not running");
  };
  const set = (id: number, patch: Partial<Row>) => {
    rows = rows.map((r) => (r.id === id ? { ...r, ...patch, speed: patch.status === "downloading" ? 5_000_000 : 0 } : r));
  };
  return {
    status: async () => status(),
    start: async () => ((running = true), status()),
    list: async () => (need(), rows.map((r) => ({ ...r }))),
    add: async (url) => {
      need();
      const id = next++;
      const name = decodeURIComponent(url.split("?")[0]!.split("/").filter(Boolean).pop() ?? "download");
      rows = [...rows, demoRow(id, name, "downloading", 0, 300_000_000, 5_000_000)];
      return id;
    },
    pause: async (id) => (need(), set(id, { status: "paused", eta: null })),
    resume: async (id) => (need(), set(id, { status: "downloading" })),
    cancel: async (id) => (need(), set(id, { status: "canceled", downloaded: 0, percent: 0, eta: null })),
    remove: async (id) => (need(), void (rows = rows.filter((r) => r.id !== id))),
  };
}

function demoRow(id: number, name: string, status: string, downloaded: number, size: number, speed: number): Row {
  return {
    id,
    name,
    url: `https://example.com/files/${name}`,
    status,
    downloaded,
    size,
    percent: (downloaded / size) * 100,
    speed,
    eta: speed > 0 ? Math.ceil((size - downloaded) / speed) : null,
    error: null,
    path: status === "completed" ? `C:\\Users\\you\\Downloads\\${name}` : null,
    expired: false,
  };
}

export function chooseApi(): Api {
  return insideTauri() ? tauriApi() : demoApi();
}
