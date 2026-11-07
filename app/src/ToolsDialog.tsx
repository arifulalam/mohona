import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import type { Api, ScanSetting, ToolInfo } from "./types";

const BROWSERS: [string, string][] = [
  ["chrome", "Chrome"],
  ["edge", "Edge"],
  ["firefox", "Firefox"],
  ["brave", "Brave"],
  ["opera", "Opera"],
  ["vivaldi", "Vivaldi"],
];

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

/** ffmpeg and yt-dlp: what each is for, whether it is found, where to get it, and where it is. */
export function ToolsDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const t = useT();
  const [tools, setTools] = useState<ToolInfo[] | null>(null);
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [browser, setBrowser] = useState<string>("");
  const [scan, setScan] = useState<ScanSetting | null>(null);
  const [command, setCommand] = useState("");

  useEffect(() => {
    let alive = true;
    api
      .tools()
      .then((list) => {
        if (!alive) return;
        setTools(list);
        setPaths(Object.fromEntries(list.map((x) => [x.name, x.chosen ?? ""])));
      })
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  useEffect(() => {
    let alive = true;
    api
      .videoBrowser()
      .then((b) => alive && setBrowser(b ?? ""))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [api]);

  useEffect(() => {
    let alive = true;
    api
      .scan()
      .then((x) => {
        if (!alive) return;
        setScan(x);
        setCommand(x.command);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [api]);

  async function chooseScan(mode: string, text: string) {
    setBusy(true);
    try {
      const x = await api.setScan(mode, text);
      setScan(x);
      setCommand(x.command);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function chooseBrowser(value: string) {
    setBusy(true);
    try {
      setBrowser((await api.setVideoBrowser(value || null)) ?? "");
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function choose(name: string, path: string | null) {
    setBusy(true);
    try {
      const list = await api.setTool(name, path);
      setTools(list);
      setPaths(Object.fromEntries(list.map((x) => [x.name, x.chosen ?? ""])));
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={t("Tools")} onClose={onClose}>
      <p className="mt-0 text-sm text-[var(--muted)]">
        {t("Mohona uses these two programs for some jobs. They are not part of Mohona: install them once, and Mohona finds them by itself, or tell it where they are.")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-[var(--bad)]">
          {error}
        </p>
      )}
      {tools === null && !error && <p>{t("Loading…")}</p>}
      <div className="flex flex-col gap-3">
        {tools?.map((tool) => (
          <section key={tool.name} aria-label={tool.name} className="rounded-lg border border-[var(--line)] p-3">
            <div className="flex items-center gap-2">
              <strong>{tool.name}</strong>
              <span className="text-sm" style={{ color: tool.found ? "var(--ok)" : "var(--bad)" }}>
                {tool.found ? t("Found") : t("Not found")}
              </span>
            </div>
            <p className="my-1 text-sm text-[var(--muted)]">
              {t("Needed to:")} {tool.purpose}
            </p>
            {tool.found && (
              <p className="my-1 break-all text-sm">
                {t("Using:")} {tool.found} {tool.chosen ? t("(your choice)") : t("(found by itself)")}
              </p>
            )}
            {!tool.found && tool.chosen && (
              <p role="alert" className="my-1 break-all text-sm text-[var(--bad)]">
                {t("Not at the place you chose:")} {tool.chosen}
              </p>
            )}
            {!tool.found && (
              <div className="my-2 text-sm">
                <div>
                  {t("Install on Windows:")} <code className="select-all rounded bg-[var(--bg)] px-1">{tool.winget}</code>
                </div>
                <div className="mt-1 flex flex-wrap gap-2">
                  <button type="button" onClick={() => void api.openToolPage(tool.windowsPage).catch((e: Error) => setError(e.message))}>
                    {t("Download for Windows")}
                  </button>
                  <button type="button" onClick={() => void api.openToolPage(tool.page).catch((e: Error) => setError(e.message))}>
                    {t("Other systems and help")}
                  </button>
                </div>
              </div>
            )}
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <input
                className={`${field} min-w-0 flex-1`}
                aria-label={`${tool.name}: ${t("Where it is")}`}
                placeholder={t("Where it is (the file, or its folder)")}
                value={paths[tool.name] ?? ""}
                onChange={(e) => setPaths({ ...paths, [tool.name]: e.target.value })}
              />
              <button type="button" disabled={busy || !(paths[tool.name] ?? "").trim()} onClick={() => void choose(tool.name, (paths[tool.name] ?? "").trim())}>
                {t("Use this")}
              </button>
              <button type="button" disabled={busy || !tool.chosen} onClick={() => void choose(tool.name, null)}>
                {t("Find it by itself")}
              </button>
            </div>
          </section>
        ))}
      </div>
      <section aria-label={t("Videos from web pages")} className="mt-3 rounded-lg border border-[var(--line)] p-3">
        <strong>{t("Videos from web pages")}</strong>
        <p className="my-1 text-sm text-[var(--muted)]">
          {t("Mohona asks yt-dlp about web pages (Facebook, X, Instagram, Pinterest, SoundCloud and many more). Posts you can only see when signed in need your browser's sign-in:")}
        </p>
        <label className="flex flex-wrap items-center gap-2 text-sm">
          {t("Use the sign-in of")}
          <select className={field} value={browser} disabled={busy} onChange={(e) => void chooseBrowser(e.target.value)}>
            <option value="">{t("No browser")}</option>
            {BROWSERS.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <p className="my-1 text-xs text-[var(--muted)]">{t("Close that browser if a download says it cannot read the cookies.")}</p>
      </section>
      {scan && (
        <section aria-label={t("Virus scan")} className="mt-3 rounded-lg border border-[var(--line)] p-3">
          <strong>{t("Virus scan")}</strong>
          <p className="my-1 text-sm text-[var(--muted)]">
            {t("Mohona can hand every finished download to a virus scanner. The download stays; a problem is shown on its row and in its log.")}
          </p>
          <label className="flex flex-wrap items-center gap-2 text-sm">
            {t("Scan finished downloads with")}
            <select className={field} value={scan.mode} disabled={busy} onChange={(e) => void chooseScan(e.target.value, command)}>
              <option value="off">{t("Nothing (off)")}</option>
              <option value="defender">{t("Windows Defender")}</option>
              <option value="custom">{t("My own program")}</option>
            </select>
          </label>
          {scan.mode === "defender" && !scan.defender && (
            <p role="alert" className="my-1 text-sm text-[var(--bad)]">
              {t("Windows Defender was not found on this computer.")}
            </p>
          )}
          {scan.mode === "custom" && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <input
                className={`${field} min-w-0 flex-1`}
                aria-label={t("Scanner command")}
                placeholder={t("For example: scan.exe --quick {file}")}
                value={command}
                onChange={(e) => setCommand(e.target.value)}
              />
              <button type="button" disabled={busy || !command.trim()} onClick={() => void chooseScan("custom", command.trim())}>
                {t("Save")}
              </button>
            </div>
          )}
          {scan.mode === "custom" && (
            <p className="my-1 text-xs text-[var(--muted)]">{t("{file} is replaced by the downloaded file. The program should end with 0 when the file is clean.")}</p>
          )}
        </section>
      )}
    </Modal>
  );
}
