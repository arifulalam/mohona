import { useEffect, useState } from "react";
import { Modal } from "./AddDialog";
import type { Api, Category } from "./types";

/** Types as typed: "zip, .7z rar" gives zip, 7z, rar. */
export function parseTypes(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((t) => t.replace(/^\./, "").toLowerCase())
    .filter(Boolean);
}

export function CategoriesDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const [list, setList] = useState<Category[] | null>(null);
  const [name, setName] = useState("");
  const [types, setTypes] = useState("");
  const [folder, setFolder] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api.categories().then((c) => alive && setList(c)).catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function change(action: () => Promise<Category[]>) {
    try {
      setList(await action());
      setError(null);
      return true;
    } catch (e) {
      setError((e as Error).message);
      return false;
    }
  }

  async function add(event: React.FormEvent) {
    event.preventDefault();
    if (await change(() => api.addCategory(name.trim(), parseTypes(types), folder.trim() || null))) {
      setName("");
      setTypes("");
      setFolder("");
    }
  }

  const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";
  return (
    <Modal title="Categories" onClose={onClose}>
      <p className="mt-0 muted">
        With “Sort finished downloads into folders by file type” on in Settings, a finished file moves into the folder of its category (inside the download folder unless you give a full path).
      </p>
      {!list ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? "Loading…"}
        </p>
      ) : (
        <>
          <ul className="m-0 max-h-60 list-none overflow-auto p-0">
            {list.map((c) => (
              <li key={c.id} className="flex items-center justify-between gap-2 border-b border-[var(--line)] py-1">
                <span className="min-w-0">
                  <strong>{c.name}</strong> <span className="muted">→ {c.folder ?? c.name}</span>
                  <div className="truncate text-[12px] muted">{c.extensions.join(", ")}</div>
                </span>
                <button className="btn !px-2 !py-0.5" aria-label={`Remove category ${c.name}`} onClick={() => void change(() => api.removeCategory(c.id))}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
          <form onSubmit={add} className="mt-3 flex flex-col gap-2">
            <div className="flex gap-2">
              <input className={`${field} w-40`} value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Category name" />
              <input className={`${field} min-w-0 flex-1`} value={types} onChange={(e) => setTypes(e.target.value)} placeholder="File types: jpg, png, gif" aria-label="File types" />
            </div>
            <span className="flex gap-2">
              <input className={`${field} min-w-0 flex-1 font-mono text-[13px]`} value={folder} onChange={(e) => setFolder(e.target.value)} placeholder="Folder (optional): a name, or a full path" aria-label="Category folder" spellCheck={false} />
              <button type="button" className="btn" onClick={() => void api.pickFolder().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
                Browse…
              </button>
            </span>
            {error && (
              <p role="alert" className="m-0 text-[var(--bad)]">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <button type="button" className="btn" onClick={onClose}>
                Close
              </button>
              <button type="submit" className="btn btn-primary">
                Add category
              </button>
            </div>
          </form>
        </>
      )}
    </Modal>
  );
}
