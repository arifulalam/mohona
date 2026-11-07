import { useEffect, useMemo, useRef, useState } from "react";
import { Modal } from "./AddDialog";
import { useT } from "./i18n";
import type { Api, GrabFile, GrabState } from "./types";

const input = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

interface Props {
  api: Api;
  queues: { id: number; name: string }[];
  defaultFolder: string;
  /** How often to ask how the run is going. */
  every?: number;
  onClose(): void;
}

/** The site grabber: look through a web page (and the pages it links to) for files, choose, download. */
export function GrabberDialog({ api, queues, defaultFolder, every = 500, onClose }: Props) {
  const t = useT();
  // The groups of files to look for (none ticked: every file).
  const KINDS: [string, string][] = [
    ["images", t("Pictures")],
    ["video", t("Videos")],
    ["audio", t("Audio")],
    ["documents", t("Documents")],
    ["archives", t("Archives")],
    ["programs", t("Programs")],
  ];
  const [url, setUrl] = useState("");
  const [depth, setDepth] = useState(1);
  const [sameSite, setSameSite] = useState(true);
  const [robots, setRobots] = useState(true);
  const [kinds, setKinds] = useState<Set<string>>(new Set());
  const [endings, setEndings] = useState("");
  const [state, setState] = useState<GrabState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [folder, setFolder] = useState("");
  const [queue, setQueue] = useState<number | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const shown = useRef<string | null>(null);

  const running = state?.state === "running";

  // While a run is going, ask how it is doing; when it is over, tick everything that was found.
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => {
      api
        .grabStatus()
        .then(setState)
        .catch((e: Error) => setError(e.message));
    }, every);
    return () => clearInterval(timer);
  }, [running, api, every]);

  useEffect(() => {
    if (!state || state.state === "running" || state.state === "idle") return;
    const key = `${state.state}:${state.found}:${state.files[0]?.url ?? ""}`;
    if (shown.current === key) return;
    shown.current = key;
    setChosen(new Set(state.files.map((f) => f.url)));
    if (state.state === "failed") setError(state.error);
  }, [state]);

  async function start(event: React.FormEvent) {
    event.preventDefault();
    const address = url.trim();
    if (!address) return setError(t("Type or paste a web address."));
    setError(null);
    setMessage(null);
    setBusy(true);
    try {
      const types = [...kinds, ...endings.split(/[\s,;]+/).filter(Boolean)];
      await api.grabStart({ url: address, depth, sameSite, types, maxPages: null, robots });
      shown.current = null;
      setChosen(new Set());
      setState(await api.grabStatus());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    try {
      await api.grabStop();
      setState(await api.grabStatus());
    } catch (e) {
      setError((e as Error).message);
    }
  }

  const files = state?.files ?? [];
  const visible = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return f ? files.filter((x) => x.name.toLowerCase().includes(f) || x.group.includes(f)) : files;
  }, [files, filter]);
  const selected = files.filter((f) => chosen.has(f.url));

  function toggle(file: GrabFile) {
    setChosen((set) => {
      const next = new Set(set);
      if (!next.delete(file.url)) next.add(file.url);
      return next;
    });
  }

  function setAll(on: boolean) {
    setChosen((set) => {
      const next = new Set(set);
      for (const f of visible) on ? next.add(f.url) : next.delete(f.url);
      return next;
    });
  }

  async function download() {
    setBusy(true);
    setError(null);
    try {
      const done = await api.grabDownload(selected, folder.trim() || null, queue);
      setMessage(
        done.skipped.length > 0
          ? `${t("Added")}: ${done.added}. ${t("Not added")}: ${done.skipped.length}\n${done.skipped.slice(0, 5).join("\n")}`
          : `${t("Added")}: ${done.added}`,
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={t("Site Grabber")} onClose={onClose} wide>
      <form onSubmit={(e) => void start(e)} className="flex flex-col gap-3">
        <p className="my-0 text-sm text-[var(--muted)]">
          {t("Looks through a web page, and the pages it links to, for files to download: pictures, videos, documents and more. Only the list is made here; nothing is downloaded until you choose.")}
        </p>
        <label className="flex flex-col gap-1">
          <span>{t("Web page")}</span>
          <input className={`${input} font-mono text-[13px]`} value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/gallery/" spellCheck={false} disabled={running} autoFocus />
        </label>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <label className="flex items-center gap-2">
            {t("How far to follow links")}
            <select className={input} value={depth} onChange={(e) => setDepth(Number(e.target.value))} disabled={running}>
              <option value={0}>{t("This page only")}</option>
              <option value={1}>{t("1 level")}</option>
              <option value={2}>{t("2 levels")}</option>
              <option value={3}>{t("3 levels")}</option>
            </select>
          </label>
          <label className="flex items-center gap-2">
            <input type="checkbox" checked={sameSite} onChange={(e) => setSameSite(e.target.checked)} disabled={running} />
            {t("Stay on this site")}
          </label>
          <label className="flex items-center gap-2">
            <input type="checkbox" checked={robots} onChange={(e) => setRobots(e.target.checked)} disabled={running} />
            {t("Obey the site's robots.txt")}
          </label>
        </div>
        <fieldset className="m-0 flex flex-wrap items-center gap-x-4 gap-y-1 border-0 p-0" disabled={running}>
          <legend className="mb-1 p-0 text-sm text-[var(--muted)]">{t("Look for (none ticked: every file)")}</legend>
          {KINDS.map(([id, name]) => (
            <label key={id} className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={kinds.has(id)}
                onChange={(e) =>
                  setKinds((k) => {
                    const next = new Set(k);
                    e.target.checked ? next.add(id) : next.delete(id);
                    return next;
                  })
                }
              />
              {name}
            </label>
          ))}
          <label className="flex items-center gap-1">
            {t("Other endings")}
            <input className={`${input} w-32`} value={endings} onChange={(e) => setEndings(e.target.value)} placeholder="pdf, epub" spellCheck={false} />
          </label>
        </fieldset>
        <div className="flex items-center gap-2">
          {running ? (
            <button type="button" className="btn" onClick={() => void stop()}>
              {t("Stop")}
            </button>
          ) : (
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {t("Look for files")}
            </button>
          )}
          <span role="status" className="min-w-0 flex-1 truncate text-sm text-[var(--muted)]">
            {running && `${t("Pages read")}: ${state?.pages ?? 0} · ${t("Files found")}: ${state?.found ?? 0}`}
            {state && state.state !== "running" && state.state !== "idle" && state.state !== "failed" && `${t("Files found")}: ${state.found} · ${t("Pages read")}: ${state.pages}`}
          </span>
        </div>
        {error && (
          <p role="alert" className="my-0 whitespace-pre-wrap text-[var(--bad)]">
            {error}
          </p>
        )}
        {state && state.state !== "running" && state.state !== "idle" && state.state !== "failed" && (
          <div className="text-sm text-[var(--muted)]">
            {state.skippedByRobots > 0 && <div>{`${state.skippedByRobots} ${t("pages were left alone because the site's robots.txt asks for it.")}`}</div>}
            {state.failedPages > 0 && <div>{`${state.failedPages} ${t("pages could not be read.")}`}</div>}
            {state.note && state.note !== "stopped" && <div>{state.note}</div>}
            {state.state === "stopped" && <div>{t("Stopped: the files found so far are listed.")}</div>}
          </div>
        )}
        {files.length > 0 && !running && (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <input className={`${input} min-w-0 flex-1`} value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("Filter the list")} aria-label={t("Filter the list")} />
              <button type="button" className="btn" onClick={() => setAll(true)}>
                {t("Select all")}
              </button>
              <button type="button" className="btn" onClick={() => setAll(false)}>
                {t("Select none")}
              </button>
            </div>
            <ul className="m-0 max-h-64 list-none overflow-auto rounded-md border border-[var(--line)] p-0" aria-label={t("Files found")}>
              {visible.map((f) => (
                <li key={f.url} className="flex items-center gap-2 border-b border-[var(--line)] px-2 py-1 last:border-b-0">
                  <input type="checkbox" checked={chosen.has(f.url)} onChange={() => toggle(f)} aria-label={f.name} />
                  <span className="min-w-0 flex-1 truncate" title={f.url}>
                    {f.name}
                  </span>
                  <span className="shrink-0 text-xs text-[var(--muted)]">{KINDS.find(([id]) => id === f.group)?.[1] ?? t("Other")}</span>
                </li>
              ))}
            </ul>
            <div className="flex flex-col gap-1">
              <span>{t("Save in")}</span>
              <span className="flex gap-2">
                <input className={`${input} min-w-0 flex-1 font-mono text-[13px]`} value={folder} onChange={(e) => setFolder(e.target.value)} placeholder={defaultFolder} aria-label={t("Save in folder")} spellCheck={false} />
                <button type="button" className="btn" onClick={() => void api.pickFolder().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
                  {t("Browse…")}
                </button>
              </span>
            </div>
            {queues.length > 1 && (
              <label className="flex items-center gap-2">
                {t("Queue")}
                <select className={input} value={queue ?? queues[0]?.id} onChange={(e) => setQueue(Number(e.target.value))} aria-label={t("Queue")}>
                  {queues.map((q) => (
                    <option key={q.id} value={q.id}>
                      {q.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {message && (
              <p role="status" className="my-0 whitespace-pre-wrap text-sm">
                {message}
              </p>
            )}
            <div className="flex items-center justify-end gap-2">
              <span className="text-sm text-[var(--muted)]">{`${selected.length} / ${files.length}`}</span>
              <button type="button" className="btn btn-primary" disabled={busy || selected.length === 0} onClick={() => void download()}>
                {t("Download selected")}
              </button>
            </div>
          </>
        )}
        {state && state.state === "done" && files.length === 0 && <p className="my-0">{t("No files found. Try one more level, or untick the kinds of files.")}</p>}
        <div className="flex justify-end">
          <button type="button" className="btn" onClick={onClose}>
            {t("Close")}
          </button>
        </div>
      </form>
    </Modal>
  );
}
