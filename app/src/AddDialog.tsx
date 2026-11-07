import { createContext, useContext, useEffect, useRef, useState } from "react";
import { bytes } from "./format";
import { useT } from "./i18n";
import type { TorrentList } from "./types";

interface Props {
  onAdd(urls: string[], folder: string | null, queue: number | null): Promise<string | null>;
  /** What is inside a torrent (a magnet link can take a while). */
  onTorrentFiles(url: string): Promise<TorrentList>;
  /** Add a torrent with only the chosen files; resolves with a problem to show, or null. */
  onAddTorrent(url: string, folder: string | null, queue: number | null, files: number[]): Promise<string | null>;
  /** The queues to choose from; the choice is shown only when there is more than one. */
  queues: { id: number; name: string }[];
  /** Opens the system's folder dialog; null when cancelled. */
  onBrowse(): Promise<string | null>;
  /** Where files go when no folder is chosen. */
  defaultFolder: string;
  onClose(): void;
}

/** A magnet link, or the address of a .torrent file. */
function isTorrent(url: string): boolean {
  return /^magnet:\?/i.test(url) || /^https?:\/\/[^?#]*\.torrent(?:[?#]|$)/i.test(url);
}

/** One address per line. */
export function AddDialog({ onAdd, onTorrentFiles, onAddTorrent, onBrowse, queues, defaultFolder, onClose }: Props) {
  const t = useT();
  const [queue, setQueue] = useState<number | null>(null);
  const [folder, setFolder] = useState("");
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const [listing, setListing] = useState<{ url: string; list: TorrentList } | null>(null);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [finding, setFinding] = useState(false);
  const lines = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  const single = lines.length === 1 && isTorrent(lines[0]!) ? lines[0]! : null;

  useEffect(() => {
    field.current?.focus();
    // A copied address is the most likely thing to add.
    navigator.clipboard
      ?.readText?.()
      .then((clip) => {
        if (/^https?:\/\/\S+$/i.test(clip.trim())) setText((t) => t || clip.trim());
      })
      .catch(() => undefined);
  }, []);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const urls = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    if (urls.length === 0) return setError(t("Type or paste an address."));
    setBusy(true);
    const problem = await onAdd(urls, folder.trim() || null, queue);
    setBusy(false);
    if (problem) setError(problem);
    else onClose();
  }

  async function findFiles() {
    if (!single) return;
    setError(null);
    setFinding(true);
    try {
      const list = await onTorrentFiles(single);
      setListing({ url: single, list });
      setChosen(new Set(list.files.map((f) => f.index)));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setFinding(false);
    }
  }

  async function addChosen() {
    if (!listing) return;
    setBusy(true);
    const everything = chosen.size === listing.list.files.length;
    const problem = everything
      ? await onAdd([listing.url], folder.trim() || null, queue)
      : await onAddTorrent(listing.url, folder.trim() || null, queue, [...chosen].sort((a, b) => a - b));
    setBusy(false);
    if (problem) setError(problem);
    else onClose();
  }

  if (listing) {
    const total = listing.list.files.filter((f) => chosen.has(f.index)).reduce((n, f) => n + f.size, 0);
    return (
      <Modal title={t("Choose files")} onClose={onClose}>
        <div className="flex flex-col gap-3">
          <p className="m-0 break-all font-semibold">{listing.list.name}</p>
          <div className="flex gap-2">
            <button type="button" className="btn" onClick={() => setChosen(new Set(listing.list.files.map((f) => f.index)))}>
              {t("Select all")}
            </button>
            <button type="button" className="btn" onClick={() => setChosen(new Set())}>
              {t("Select none")}
            </button>
          </div>
          <ul className="m-0 max-h-64 list-none overflow-y-auto overflow-x-hidden rounded-md border border-[var(--line)] p-1">
            {listing.list.files.map((f) => (
              <li key={f.index}>
                <label className="flex items-start gap-2 p-1">
                  <input
                    type="checkbox"
                    checked={chosen.has(f.index)}
                    onChange={(e) =>
                      setChosen((c) => {
                        const next = new Set(c);
                        if (e.target.checked) next.add(f.index);
                        else next.delete(f.index);
                        return next;
                      })
                    }
                  />
                  <span className="min-w-0 flex-1 break-all font-mono text-[13px]">{f.path}</span>
                  <span className="shrink-0 text-[var(--muted)]">{bytes(f.size)}</span>
                </label>
              </li>
            ))}
          </ul>
          <p className="m-0">
            {t("Selected")}: {chosen.size}/{listing.list.files.length} · {bytes(total)}
          </p>
          {error && (
            <p role="alert" className="whitespace-pre-wrap text-[var(--bad)]">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <button type="button" className="btn" onClick={() => (setListing(null), setError(null))}>
              {t("Back")}
            </button>
            <button type="button" className="btn btn-primary" disabled={busy || chosen.size === 0} onClick={() => void addChosen()}>
              {busy ? t("Adding…") : t("Download")}
            </button>
          </div>
        </div>
      </Modal>
    );
  }

  return (
    <Modal title={t("Add downloads")} onClose={onClose}>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1">
          <span>{t("Addresses (one per line)")}</span>
          <textarea
            ref={field}
            value={text}
            onChange={(e) => setText(e.target.value)}
            rows={5}
            placeholder="https://example.com/file.zip"
            className="rounded-md border border-[var(--line)] bg-[var(--bg)] p-2 font-mono text-[13px]"
            spellCheck={false}
          />
        </label>
        <div className="flex flex-col gap-1">
          <span>{t("Save in")}</span>
          <span className="flex gap-2">
            <input
              className="min-w-0 flex-1 rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1 font-mono text-[13px]"
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
              placeholder={defaultFolder}
              aria-label={t("Save in folder")}
              spellCheck={false}
            />
            <button type="button" className="btn" onClick={() => void onBrowse().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
              {t("Browse…")}
            </button>
          </span>
        </div>
        {queues.length > 1 && (
          <label className="flex items-center gap-2">
            <span>{t("Queue")}</span>
            <select
              className="rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1"
              value={queue ?? queues[0]?.id}
              onChange={(e) => setQueue(Number(e.target.value))}
              aria-label={t("Queue")}
            >
              {queues.map((q) => (
                <option key={q.id} value={q.id}>
                  {q.name}
                </option>
              ))}
            </select>
          </label>
        )}
        {error && (
          <p role="alert" className="whitespace-pre-wrap text-[var(--bad)]">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button type="button" className="btn" onClick={onClose}>
            {t("Cancel")}
          </button>
          {single && (
            <button type="button" className="btn" disabled={busy || finding} onClick={() => void findFiles()}>
              {finding ? t("Finding the files…") : t("Choose files…")}
            </button>
          )}
          <button type="submit" className="btn btn-primary" disabled={busy || finding}>
            {busy ? t("Adding…") : t("Download")}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Inside the settings window a dialog shows only its content: the window has the frame. */
export const Embedded = createContext(false);

export function Modal({ title, onClose, children, wide }: { title: string; onClose(): void; children: React.ReactNode; wide?: boolean }) {
  const embedded = useContext(Embedded);
  useEffect(() => {
    if (embedded) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, embedded]);
  if (embedded) return <div className="flex min-w-0 flex-col">{children}</div>;
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={title} className={`w-full ${wide ? "max-w-3xl" : "max-w-lg"} max-h-[90vh] overflow-auto rounded-xl border border-[var(--line)] bg-[var(--panel)] p-5 shadow-xl`}>
        <h2 className="mb-3 text-base font-semibold">{title}</h2>
        {children}
      </div>
    </div>
  );
}
