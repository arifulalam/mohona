import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import { parseLimit, splitLimit, type Unit } from "./limits";
import { WindowsEditor } from "./WindowsEditor";
import type { Api, PlanWindow, Queue } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

export function QueuesDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const t = useT();
  const [list, setList] = useState<Queue[] | null>(null);
  const [current, setCurrent] = useState<number | null>(null);
  const [newName, setNewName] = useState("");
  const [name, setName] = useState("");
  const [active, setActive] = useState("3");
  const [limit, setLimit] = useState("");
  const [unit, setUnit] = useState<Unit>("MB");
  const [windows, setWindows] = useState<PlanWindow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function open(queue: Queue | undefined) {
    if (!queue) return;
    setCurrent(queue.id);
    setName(queue.name);
    setActive(String(queue.maxActive));
    const l = splitLimit(queue.speedLimit);
    setLimit(l.text);
    setUnit(l.unit);
    setWindows(queue.windows);
  }

  useEffect(() => {
    let alive = true;
    api
      .queues()
      .then((q) => {
        if (!alive) return;
        setList(q);
        open(q[0]);
      })
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function change(action: () => Promise<Queue[]>, then?: (q: Queue[]) => void) {
    setBusy(true);
    try {
      const q = await action();
      setList(q);
      setError(null);
      then?.(q);
      return true;
    } catch (e) {
      setError((e as Error).message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function add(event: React.FormEvent) {
    event.preventDefault();
    if (await change(() => api.addQueue(newName), (q) => open(q[q.length - 1]))) setNewName("");
  }

  async function save(event: React.FormEvent) {
    event.preventDefault();
    const queue = list?.find((q) => q.id === current);
    if (!queue) return;
    const simultaneous = Number(active);
    const speed = parseLimit(limit, unit);
    if (speed === null) return setError(t("The speed limit must be a number, or empty for no limit."));
    if (!Number.isInteger(simultaneous) || simultaneous < 1 || simultaneous > 32) return setError(t("Simultaneous downloads must be a whole number from 1 to 32."));
    const ok = await change(async () => {
      if (name.trim() !== queue.name) await api.renameQueue(queue.id, name);
      return api.setQueue(queue.id, { maxActive: simultaneous, speedLimit: speed, windows });
    });
    if (ok) onClose();
  }

  const queue = list?.find((q) => q.id === current);
  const isMain = list?.[0]?.id === current;
  return (
    <Modal title={t("Queues")} onClose={onClose}>
      {!list ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? t("Loading…")}
        </p>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="flex gap-4">
            <div className="flex w-44 shrink-0 flex-col gap-2">
              <ul role="listbox" aria-label={t("Queues")} className="m-0 list-none p-0">
                {list.map((q) => (
                  <li key={q.id} role="option" aria-selected={q.id === current}>
                    <button type="button" onClick={() => open(q)} className="w-full rounded-md border-0 bg-transparent px-2 py-1 text-left hover:bg-[var(--hover)]" style={q.id === current ? { background: "var(--selected)", fontWeight: 600 } : undefined}>
                      {q.name}
                      {!q.openNow && <span className="muted">{t("(waiting)")}</span>}
                    </button>
                  </li>
                ))}
              </ul>
              <form onSubmit={add} className="flex gap-1">
                <input className={`${field} min-w-0 flex-1`} value={newName} onChange={(e) => setNewName(e.target.value)} placeholder={t("New queue")} aria-label={t("New queue name")} />
                <button type="submit" className="btn !px-2" disabled={busy || !newName.trim()}>
                  {t("Add")}
                </button>
              </form>
            </div>
            {queue && (
              <form onSubmit={save} className="flex min-w-0 flex-1 flex-col gap-3">
                <div className="flex max-h-[50vh] flex-col gap-3 overflow-auto pr-1">
                  <label className="flex flex-col gap-1">
                    <span>{t("Name")}</span>
                    <input className={field} value={name} onChange={(e) => setName(e.target.value)} aria-label={t("Queue name")} />
                  </label>
                  <div className="flex flex-wrap items-end gap-4">
                    <label className="flex flex-col gap-1">
                      <span>{t("Downloads at once")}</span>
                      <input className={`${field} w-20`} value={active} onChange={(e) => setActive(e.target.value)} aria-label={t("Queue simultaneous downloads")} />
                    </label>
                    <label className="flex flex-col gap-1">
                      <span>{t("Speed limit (this queue)")}</span>
                      <span className="flex gap-1">
                        <input className={`${field} w-24`} value={limit} onChange={(e) => setLimit(e.target.value)} placeholder={t("no limit")} aria-label={t("Queue speed limit")} />
                        <select className={field} value={unit} onChange={(e) => setUnit(e.target.value as Unit)} aria-label={t("Queue speed unit")}>
                          <option value="KB">{t("KB/s")}</option>
                          <option value="MB">{t("MB/s")}</option>
                        </select>
                      </span>
                    </label>
                  </div>
                  <WindowsEditor windows={windows} onChange={setWindows} onError={setError} />
                </div>
                <div className="flex justify-between gap-2">
                  <button
                    type="button"
                    className="btn btn-danger"
                    disabled={busy || isMain}
                    title={isMain ? t("The main queue cannot be removed") : t("Its downloads move to the main queue")}
                    onClick={() => void change(() => api.removeQueue(queue.id), (q) => open(q[0]))}
                  >
                    {t("Remove queue")}
                  </button>
                  <span className="flex gap-2">
                    <button type="button" className="btn" onClick={onClose}>
                      {t("Cancel")}
                    </button>
                    <button type="submit" className="btn btn-primary" disabled={busy}>
                      {t("Save")}
                    </button>
                  </span>
                </div>
              </form>
            )}
          </div>
          {error && (
            <p role="alert" className="m-0 text-[var(--bad)]">
              {error}
            </p>
          )}
        </div>
      )}
    </Modal>
  );
}
