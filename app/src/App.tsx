import { useEffect, useMemo, useState } from "react";
import { AddDialog, Modal } from "./AddDialog";
import { allowed, counts, FILTERS, matches, type Filter } from "./filters";
import { List } from "./List";
import { applyTheme, loadTheme, nextTheme, saveTheme, type Theme } from "./theme";
import type { Api } from "./types";
import { useDownloads } from "./useDownloads";

const THEME_LABEL: Record<Theme, string> = { system: "Theme: system", light: "Theme: light", dark: "Theme: dark" };

export function App({ api, every }: { api: Api; every?: number }) {
  const d = useDownloads(api, every);
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [theme, setTheme] = useState<Theme>(loadTheme());

  useEffect(() => {
    applyTheme(theme);
    saveTheme(theme);
  }, [theme]);

  const shown = useMemo(() => d.rows.filter((r) => matches(r, filter)), [d.rows, filter]);
  const number = useMemo(() => counts(d.rows), [d.rows]);
  const chosen = useMemo(() => d.rows.filter((r) => selected.has(r.id)), [d.rows, selected]);
  const can = allowed(chosen);
  const running = d.engine?.running === true;

  // Forget selected rows that are gone.
  useEffect(() => {
    setSelected((s) => {
      const keep = new Set([...s].filter((id) => d.rows.some((r) => r.id === id)));
      return keep.size === s.size ? s : keep;
    });
  }, [d.rows]);

  function select(id: number, additive: boolean) {
    setSelected((s) => {
      if (!additive) return s.size === 1 && s.has(id) ? new Set() : new Set([id]);
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  const each = (rows: typeof chosen, call: (id: number) => Promise<void>) => d.act(() => Promise.all(rows.map((r) => call(r.id))));

  async function addAll(urls: string[]): Promise<string | null> {
    const problems: string[] = [];
    for (const url of urls) {
      try {
        await api.add(url, null);
      } catch (e) {
        problems.push(`${url}: ${(e as Error).message}`);
      }
    }
    await d.act(async () => undefined);
    return problems.length ? problems.join("\n") : null;
  }

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 border-b border-[var(--line)] bg-[var(--panel)] px-4 py-2">
        <h1 className="m-0 text-base font-semibold">Mohona</h1>
        <span className="text-[12px] muted" aria-live="polite">
          {d.engine === null ? "Connecting…" : running ? `Engine running${d.engine.version ? ` (${d.engine.version})` : ""}` : "Engine stopped"}
        </span>
        <div className="flex-1" />
        <button className="btn" onClick={() => setTheme(nextTheme(theme))} title="Change the colour theme">
          {THEME_LABEL[theme]}
        </button>
      </header>

      {!running && d.engine !== null && (
        <div role="status" className="flex items-center gap-3 border-b border-[var(--line)] bg-[var(--selected)] px-4 py-2">
          <span>Mohona is not running, so downloads cannot start.</span>
          {d.engine.message && <span className="muted">{d.engine.message}</span>}
          <button className="btn btn-primary" onClick={() => void d.startEngine()}>
            Start Mohona
          </button>
        </div>
      )}
      {d.error && (
        <div role="alert" className="flex items-center gap-3 border-b border-[var(--line)] px-4 py-2 text-[var(--bad)]">
          <span className="flex-1 whitespace-pre-wrap">{d.error}</span>
          <button className="btn" onClick={d.dismissError}>
            Dismiss
          </button>
        </div>
      )}

      <div className="flex items-center gap-2 border-b border-[var(--line)] bg-[var(--panel)] px-4 py-2" role="toolbar" aria-label="Actions">
        <button className="btn btn-primary" onClick={() => setAdding(true)} disabled={!running}>
          + Add
        </button>
        <button className="btn" disabled={!can.pause} onClick={() => void each(chosen, api.pause)}>
          Pause
        </button>
        <button className="btn" disabled={!can.resume} onClick={() => void each(chosen, api.resume)}>
          Resume
        </button>
        <button className="btn" disabled={!can.cancel} onClick={() => void each(chosen, api.cancel)}>
          Cancel
        </button>
        <button className="btn btn-danger" disabled={!can.remove} onClick={() => (setDeleteFiles(false), setRemoving(true))}>
          Remove
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        <nav aria-label="Filters" className="w-48 shrink-0 border-r border-[var(--line)] bg-[var(--panel)] p-2">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              onClick={() => setFilter(f.id)}
              aria-current={filter === f.id ? "page" : undefined}
              className="flex w-full items-center justify-between rounded-md border-0 bg-transparent px-3 py-2 text-left hover:bg-[var(--hover)] aria-[current=page]:bg-[var(--selected)] aria-[current=page]:font-semibold"
            >
              <span>{f.label}</span>
              <span className="muted">{number[f.id]}</span>
            </button>
          ))}
        </nav>
        <main className="min-w-0 flex-1 overflow-auto bg-[var(--panel)]">
          {shown.length === 0 ? (
            <div className="p-10 text-center muted">
              {d.rows.length === 0 ? (running ? "No downloads yet. Press “+ Add” and paste an address." : "Start Mohona to see your downloads.") : "Nothing here."}
            </div>
          ) : (
            <List rows={shown} selected={selected} onSelect={select} onPause={(id) => void d.act(() => api.pause(id))} onResume={(id) => void d.act(() => api.resume(id))} />
          )}
        </main>
      </div>

      {adding && <AddDialog onAdd={addAll} onClose={() => setAdding(false)} />}
      {removing && (
        <Modal title="Remove from the list" onClose={() => setRemoving(false)}>
          <p className="mt-0">
            Remove {chosen.length === 1 ? `“${chosen[0]!.name}”` : `${chosen.length} downloads`} from the list?
          </p>
          <label className="mb-4 flex items-center gap-2">
            <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
            Also delete the downloaded file{chosen.length === 1 ? "" : "s"} from the disk
          </label>
          <div className="flex justify-end gap-2">
            <button className="btn" onClick={() => setRemoving(false)}>
              Keep
            </button>
            <button
              className="btn btn-primary"
              onClick={async () => {
                const ok = await each(chosen, (id) => api.remove(id, deleteFiles));
                if (ok) setSelected(new Set());
                setRemoving(false);
              }}
            >
              Remove
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
