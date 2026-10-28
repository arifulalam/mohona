import { useEffect, useRef, useState } from "react";

interface Props {
  onAdd(urls: string[], folder: string | null): Promise<string | null>;
  /** Opens the system's folder dialog; null when cancelled. */
  onBrowse(): Promise<string | null>;
  /** Where files go when no folder is chosen. */
  defaultFolder: string;
  onClose(): void;
}

/** One address per line. */
export function AddDialog({ onAdd, onBrowse, defaultFolder, onClose }: Props) {
  const [folder, setFolder] = useState("");
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);

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
    if (urls.length === 0) return setError("Type or paste an address.");
    setBusy(true);
    const problem = await onAdd(urls, folder.trim() || null);
    setBusy(false);
    if (problem) setError(problem);
    else onClose();
  }

  return (
    <Modal title="Add downloads" onClose={onClose}>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1">
          <span>Addresses (one per line)</span>
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
          <span>Save in</span>
          <span className="flex gap-2">
            <input
              className="min-w-0 flex-1 rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1 font-mono text-[13px]"
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
              placeholder={defaultFolder}
              aria-label="Save in folder"
              spellCheck={false}
            />
            <button type="button" className="btn" onClick={() => void onBrowse().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
              Browse…
            </button>
          </span>
        </div>
        {error && (
          <p role="alert" className="whitespace-pre-wrap text-[var(--bad)]">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? "Adding…" : "Download"}
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function Modal({ title, onClose, children }: { title: string; onClose(): void; children: React.ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={title} className="w-full max-w-lg rounded-xl border border-[var(--line)] bg-[var(--panel)] p-5 shadow-xl">
        <h2 className="mb-3 text-base font-semibold">{title}</h2>
        {children}
      </div>
    </div>
  );
}
