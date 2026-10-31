import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import { parseHosts, parseLimit, parseMegabytes, splitLimit, type Unit } from "./limits";
import type { Api, Config, ExtensionSettings } from "./types";

export function SettingsDialog({ api, onClose, clipboard, onClipboard }: { api: Api; onClose(): void; clipboard: boolean; onClipboard(on: boolean): void }) {
  const t = useT();
  const [config, setConfig] = useState<Config | null>(null);
  const [ext, setExt] = useState<ExtensionSettings | null>(null);
  const [folder, setFolder] = useState("");
  const [limit, setLimit] = useState("");
  const [unit, setUnit] = useState<Unit>("MB");
  const [active, setActive] = useState("4");
  const [minMb, setMinMb] = useState("");
  const [excluded, setExcluded] = useState("");
  const [notify, setNotify] = useState(true);
  const [tray, setTray] = useState(true);
  const [sort, setSort] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    Promise.all([api.config(), api.extensionSettings()])
      .then(([c, e]) => {
        if (!alive) return;
        setConfig(c);
        setExt(e);
        setFolder(c.downloadFolder);
        const l = splitLimit(c.speedLimit);
        setLimit(l.text);
        setUnit(l.unit);
        setActive(String(c.maxActive));
        setNotify(c.notifications);
        setTray(c.keepInTray);
        setSort(c.sortByType);
        setMinMb(e.minSizeBytes ? String(Math.round(e.minSizeBytes / 1024 / 1024)) : "");
        setExcluded(e.excludedHosts.join("\n"));
      })
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!config || !ext) return;
    const speed = parseLimit(limit, unit);
    const simultaneous = Number(active);
    const min = parseMegabytes(minMb);
    if (speed === null) return setError(t("The speed limit must be a number, or empty for no limit."));
    if (!Number.isInteger(simultaneous) || simultaneous < 1 || simultaneous > 32) return setError(t("Simultaneous downloads must be a whole number from 1 to 32."));
    if (min === null) return setError(t("The smallest size to take over must be a whole number of MB."));
    setBusy(true);
    try {
      await api.setConfig({
        downloadFolder: folder.trim() === config.defaultFolder ? "" : folder.trim(),
        speedLimit: speed,
        maxActive: simultaneous,
        notifications: notify,
        keepInTray: tray,
        sortByType: sort,
      });
      await api.setExtensionSettings({ ...ext, minSizeBytes: min, excludedHosts: parseHosts(excluded) });
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";
  return (
    <Modal title={t("Settings")} onClose={onClose}>
      {!config || !ext ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? t("Loading…")}
        </p>
      ) : (
        <form onSubmit={save} className="flex flex-col gap-4">
          <div className="flex max-h-[60vh] flex-col gap-4 overflow-auto pr-1">
          <fieldset className="m-0 flex flex-col gap-3 border-0 p-0">
            <legend className="mb-1 font-semibold">{t("Downloads")}</legend>
            <label className="flex flex-col gap-1">
              <span>{t("Save files in")}</span>
              <span className="flex gap-2">
                <input className={`${field} min-w-0 flex-1 font-mono text-[13px]`} value={folder} onChange={(e) => setFolder(e.target.value)} spellCheck={false} aria-label={t("Download folder")} />
                <button type="button" className="btn" onClick={() => void api.pickFolder().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
                  {t("Browse…")}
                </button>
              </span>
              <small className="muted">A full path. Empty goes back to {config.defaultFolder}.</small>
            </label>
            <div className="flex flex-wrap items-end gap-4">
              <label className="flex flex-col gap-1">
                <span>{t("Speed limit (all downloads)")}</span>
                <span className="flex gap-1">
                  <input className={`${field} w-24`} inputMode="decimal" value={limit} onChange={(e) => setLimit(e.target.value)} placeholder={t("no limit")} aria-label={t("Speed limit")} />
                  <select className={field} value={unit} onChange={(e) => setUnit(e.target.value as Unit)} aria-label={t("Speed unit")}>
                    <option value="KB">{t("KB/s")}</option>
                    <option value="MB">{t("MB/s")}</option>
                  </select>
                </span>
              </label>
              <label className="flex flex-col gap-1">
                <span>{t("Downloads at the same time")}</span>
                <input className={`${field} w-20`} inputMode="numeric" value={active} onChange={(e) => setActive(e.target.value)} aria-label={t("Simultaneous downloads")} />
              </label>
            </div>
            <Check label={t("Sort finished downloads into folders by file type (see Categories)")} value={sort} onChange={setSort} />
            <Check label={t("Show a message when a download finishes or fails")} value={notify} onChange={setNotify} />
            <Check label={t("Keep Mohona in the tray when the window is closed")} value={tray} onChange={setTray} />
            <Check label={t("Offer to download a link when I copy one")} value={clipboard} onChange={onClipboard} />
          </fieldset>

          <fieldset className="m-0 flex flex-col gap-3 border-0 p-0">
            <legend className="mb-1 font-semibold">{t("Browsers")}</legend>
            <small className="muted">{t("Shared with the Mohona extension in every browser.")}</small>
            <Check label={t("Take over downloads from the browser")} value={ext.intercept} onChange={(v) => setExt({ ...ext, intercept: v })} />
            <Check label={t("Show a download button on videos")} value={ext.videoButton} onChange={(v) => setExt({ ...ext, videoButton: v })} />
            <Check label={t("Renew expired download links by themselves")} value={ext.autoRefresh} onChange={(v) => setExt({ ...ext, autoRefresh: v })} />
            <label className="flex items-center gap-2">
              {t("Leave files smaller than")}
              <input className={`${field} w-20`} inputMode="numeric" value={minMb} onChange={(e) => setMinMb(e.target.value)} aria-label={t("Smallest size in MB")} placeholder="0" />
              {t("MB to the browser")}
            </label>
            <label className="flex flex-col gap-1">
              <span>{t("Never take downloads from these sites (one per line)")}</span>
              <textarea className={`${field} font-mono text-[13px]`} rows={3} value={excluded} onChange={(e) => setExcluded(e.target.value)} placeholder="example.com" spellCheck={false} />
            </label>
          </fieldset>
          </div>

          {error && (
            <p role="alert" className="m-0 text-[var(--bad)]">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <button type="button" className="btn" onClick={onClose}>
              {t("Cancel")}
            </button>
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {busy ? t("Saving…") : t("Save")}
            </button>
          </div>
        </form>
      )}
    </Modal>
  );
}

function Check({ label, value, onChange }: { label: string; value: boolean; onChange(v: boolean): void }) {
  return (
    <label className="flex items-center gap-2">
      <input type="checkbox" checked={value} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}
