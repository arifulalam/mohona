import { useEffect, useMemo, useState } from "react";
import { I18n, label, LANGUAGES, loadLanguage, saveLanguage, translator, useT } from "./i18n";
import { AddDialog, Modal } from "./AddDialog";
import { CategoriesDialog } from "./CategoriesDialog";
import { loadClipboardWatch, saveClipboardWatch, useClipboardOffer } from "./clipboard";
import { DetailsDialog } from "./DetailsDialog";
import { categoryCounts, categoryOf, SORTS, sortRows, type SortKey } from "./organize";
import { allowed, counts, FILTERS, matches, type Filter } from "./filters";
import { List } from "./List";
import { QueuesDialog } from "./QueuesDialog";
import { RulesDialog } from "./RulesDialog";
import { ProfilesDialog } from "./ProfilesDialog";
import { ScheduleDialog } from "./ScheduleDialog";
import { actionName } from "./schedule";
import { SettingsDialog } from "./SettingsDialog";
import { applyTheme, loadTheme, saveTheme, THEMES, type Theme } from "./theme";
import type { Api, Category } from "./types";
import { useDownloads } from "./useDownloads";

function loadSort(): SortKey {
  try {
    const v = localStorage.getItem("mohona.sort");
    if (SORTS.some((s) => s.id === v)) return v as SortKey;
  } catch {
    /* storage may be unavailable */
  }
  return "added";
}

/** The window, with the language the person chose. */
export function App(props: { api: Api; every?: number }) {
  const [code, setCode] = useState(() => loadLanguage().code);
  const lang = LANGUAGES.find((l) => l.code === code) ?? LANGUAGES[0]!;
  const t = useMemo(() => translator(lang.dict), [lang]);
  useEffect(() => {
    document.documentElement.lang = lang.code;
    document.documentElement.dir = lang.dir;
    saveLanguage(lang.code);
  }, [lang]);
  return (
    <I18n.Provider value={t}>
      <Main {...props} language={lang.code} onLanguage={setCode} />
    </I18n.Provider>
  );
}

function Main({ api, every, language, onLanguage }: { api: Api; every?: number; language: string; onLanguage(code: string): void }) {
  const t = useT();
  const d = useDownloads(api, every);
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [settings, setSettings] = useState(false);
  const [schedule, setSchedule] = useState(false);
  const [queuesOpen, setQueuesOpen] = useState(false);
  const [rulesOpen, setRulesOpen] = useState(false);
  const [profilesOpen, setProfilesOpen] = useState(false);
  const [queueList, setQueueList] = useState<{ id: number; name: string }[]>([]);
  const [categories, setCategories] = useState(false);
  const [cats, setCats] = useState<Category[]>([]);
  const [category, setCategory] = useState<string | null>(null);
  const [sort, setSort] = useState<SortKey>(loadSort());
  const [detailsId, setDetailsId] = useState<number | null>(null);
  const [watchClipboard, setWatchClipboard] = useState(loadClipboardWatch());
  const [defaultFolder, setDefaultFolder] = useState("");
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [theme, setTheme] = useState<Theme>(loadTheme());

  useEffect(() => {
    applyTheme(theme);
    saveTheme(theme);
  }, [theme]);

  const running = d.engine?.running === true;
  useEffect(() => {
    if (running && !categories) api.categories().then(setCats).catch(() => undefined);
  }, [api, running, categories]);
  useEffect(() => {
    try {
      localStorage.setItem("mohona.sort", sort);
    } catch {
      /* ignore */
    }
  }, [sort]);

  const clip = useClipboardOffer(api, watchClipboard, d.rows, running);
  useEffect(() => saveClipboardWatch(watchClipboard), [watchClipboard]);

  const shown = useMemo(
    () =>
      sortRows(
        d.rows.filter((r) => matches(r, filter) && (category === null || categoryOf(r.name, cats) === category)),
        sort,
        cats,
      ),
    [d.rows, filter, category, sort, cats],
  );
  const inFilter = useMemo(() => d.rows.filter((r) => matches(r, filter)), [d.rows, filter]);
  const byCategory = useMemo(() => categoryCounts(inFilter, cats), [inFilter, cats]);
  const number = useMemo(() => counts(d.rows), [d.rows]);
  const chosen = useMemo(() => d.rows.filter((r) => selected.has(r.id)), [d.rows, selected]);
  const can = allowed(chosen);

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

  async function addAll(urls: string[], folder: string | null, queue: number | null): Promise<string | null> {
    const problems: string[] = [];
    for (const url of urls) {
      try {
        await (queue === null ? api.add(url, folder) : api.add(url, folder, queue));
      } catch (e) {
        problems.push(`${url}: ${(e as Error).message}`);
      }
    }
    await d.act(async () => undefined);
    return problems.length ? problems.join("\n") : null;
  }

  return (
    <div className="flex h-full flex-col">
      <header className="app-header flex items-center gap-3 border-b border-[var(--line)] px-4 py-2">
        <h1 className="m-0 text-base font-semibold">{t("Mohona")}</h1>
        <span className="status text-[12px]" aria-live="polite">
          {d.engine === null ? t("Connecting…") : running ? `${t("Engine running")}${d.engine.version ? ` (${d.engine.version})` : ""}` : t("Engine stopped")}
        </span>
        <div className="flex-1" />
        <button className="btn" onClick={() => setCategories(true)} disabled={!running}>
          {t("Categories")}
        </button>
        <button className="btn" onClick={() => setQueuesOpen(true)} disabled={!running}>
          {t("Queues")}
        </button>
        <button className="btn" onClick={() => setRulesOpen(true)} disabled={!running}>
          {t("Rules")}
        </button>
        <button className="btn" onClick={() => setProfilesOpen(true)} disabled={!running}>
          {t("Site profiles")}
        </button>
        <button className="btn" onClick={() => setSchedule(true)} disabled={!running}>
          {t("Schedule")}
        </button>
        <button className="btn" onClick={() => setSettings(true)} disabled={!running}>
          {t("Settings")}
        </button>
        <select className="btn" value={language} onChange={(e) => onLanguage(e.target.value)} aria-label={t("Language")} title={t("Language")}>
          {LANGUAGES.map((l) => (
            <option key={l.code} value={l.code} lang={l.code}>
              {label(l)}
            </option>
          ))}
        </select>
        <select className="btn" value={theme} onChange={(e) => setTheme(e.target.value as Theme)} aria-label={t("Colour theme")} title={t("Colour theme")}>
          {THEMES.map((th) => (
            <option key={th.id} value={th.id}>
              {t(th.label)}
            </option>
          ))}
        </select>
      </header>

      {!running && d.engine !== null && (
        <div role="status" className="flex items-center gap-3 border-b border-[var(--line)] bg-[var(--selected)] px-4 py-2">
          <span>{t("Mohona is not running, so downloads cannot start.")}</span>
          {d.engine.message && <span className="muted">{d.engine.message}</span>}
          <button className="btn btn-primary" onClick={() => void d.startEngine()}>
            {t("Start Mohona")}
          </button>
        </div>
      )}
      {d.plan?.windows.length ? (
        !d.plan.openNow && (
          <div role="status" className="border-b border-[var(--line)] px-4 py-1 text-[12px] muted">
            {t("Outside the scheduled hours: downloads wait for the next window.")}
          </div>
        )
      ) : null}
      {d.plan?.action && (
        <div role="status" className="flex items-center gap-3 border-b border-[var(--line)] bg-[var(--selected)] px-4 py-2">
          <span>
            {d.plan.countdown !== null
              ? t("{action} in {seconds} s.", { action: t(actionName(d.plan.action)), seconds: d.plan.countdown })
              : t("When all downloads are done: {action}.", { action: t(actionName(d.plan.action)).toLowerCase() })}
          </span>
          <button className="btn" onClick={() => void d.act(() => api.cancelAction())}>
            {t("Cancel")}
          </button>
        </div>
      )}
      {clip.offer && (
        <div role="status" className="flex items-center gap-3 border-b border-[var(--line)] bg-[var(--selected)] px-4 py-2">
          <span className="min-w-0 flex-1 truncate" title={clip.offer}>
            {t("You copied a link: {link}", { link: clip.offer })}
          </span>
          <button
            className="btn btn-primary"
            onClick={() => {
              const link = clip.offer;
              clip.dismiss();
              if (link) void d.act(() => api.add(link, null));
            }}
          >
            {t("Download it")}
          </button>
          <button className="btn" onClick={clip.dismiss}>
            {t("Ignore")}
          </button>
        </div>
      )}
      {d.plan?.message && !d.plan.action && (
        <div role="status" className="border-b border-[var(--line)] px-4 py-1 text-[12px] muted">
          {d.plan.message}
        </div>
      )}
      {d.error && (
        <div role="alert" className="flex items-center gap-3 border-b border-[var(--line)] px-4 py-2 text-[var(--bad)]">
          <span className="flex-1 whitespace-pre-wrap">{d.error}</span>
          <button className="btn" onClick={d.dismissError}>
            {t("Dismiss")}
          </button>
        </div>
      )}

      <div className="flex items-center gap-2 border-b border-[var(--line)] bg-[var(--panel)] px-4 py-2" role="toolbar" aria-label={t("Actions")}>
        <button
          className="btn btn-primary"
          onClick={() => {
            setAdding(true);
            api.config().then((c) => setDefaultFolder(c.downloadFolder)).catch(() => undefined);
            api.queues().then(setQueueList).catch(() => undefined);
          }}
          disabled={!running}
        >
          {t("+ Add")}
        </button>
        <button className="btn" disabled={!can.pause} onClick={() => void each(chosen, api.pause)}>
          {t("Pause")}
        </button>
        <button className="btn" disabled={!can.resume} onClick={() => void each(chosen, api.resume)}>
          {t("Resume")}
        </button>
        <button className="btn" disabled={!can.cancel} onClick={() => void each(chosen, api.cancel)}>
          {t("Cancel")}
        </button>
        <button className="btn btn-danger" disabled={!can.remove} onClick={() => (setDeleteFiles(false), setRemoving(true))}>
          {t("Remove")}
        </button>
        <div className="flex-1" />
        <label className="flex items-center gap-2 text-[12px] muted">
          {t("Sort by")}
          <select className="btn !py-0.5" value={sort} onChange={(e) => setSort(e.target.value as SortKey)} aria-label={t("Sort by")}>
            {SORTS.map((o) => (
              <option key={o.id} value={o.id}>
                {t(o.label)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="flex min-h-0 flex-1">
        <nav aria-label={t("Filters")} className="w-48 shrink-0 border-r border-[var(--line)] bg-[var(--panel)] p-2">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              onClick={() => setFilter(f.id)}
              aria-current={filter === f.id ? "page" : undefined}
              className="flex w-full items-center justify-between rounded-md border-0 bg-transparent px-3 py-2 text-left hover:bg-[var(--hover)] aria-[current=page]:bg-[var(--selected)] aria-[current=page]:font-semibold"
            >
              <span>{t(f.label)}</span>
              <span className="muted">{number[f.id]}</span>
            </button>
          ))}
          <h2 className="mb-1 mt-4 px-3 text-[11px] font-semibold uppercase tracking-wide muted">{t("Categories")}</h2>
          <button
            onClick={() => setCategory(null)}
            aria-current={category === null ? "true" : undefined}
            className="flex w-full items-center justify-between rounded-md border-0 bg-transparent px-3 py-1.5 text-left hover:bg-[var(--hover)] aria-[current=true]:bg-[var(--selected)] aria-[current=true]:font-semibold"
          >
            <span>{t("All types")}</span>
            <span className="muted">{inFilter.length}</span>
          </button>
          {byCategory.map((c) => (
            <button
              key={c.name}
              onClick={() => setCategory(category === c.name ? null : c.name)}
              aria-current={category === c.name ? "true" : undefined}
              aria-label={t("Category {name}, {count}", { name: t(c.name), count: c.count })}
              className="flex w-full items-center justify-between rounded-md border-0 bg-transparent px-3 py-1.5 text-left hover:bg-[var(--hover)] aria-[current=true]:bg-[var(--selected)] aria-[current=true]:font-semibold"
            >
              <span className="truncate">{t(c.name)}</span>
              <span className="muted">{c.count}</span>
            </button>
          ))}
        </nav>
        <main className="min-w-0 flex-1 overflow-auto bg-[var(--panel)]">
          {shown.length === 0 ? (
            <div className="p-10 text-center muted">
              {d.rows.length === 0 ? (running ? t("No downloads yet. Press “+ Add” and paste an address.") : t("Start Mohona to see your downloads.")) : t("Nothing here.")}
            </div>
          ) : (
            <List rows={shown} selected={selected} onSelect={select} onPause={(id) => void d.act(() => api.pause(id))} onResume={(id) => void d.act(() => api.resume(id))} onOpen={(id) => void d.act(() => api.open(id))} onShow={(id) => void d.act(() => api.show(id))} onDetails={setDetailsId} category={(r) => categoryOf(r.name, cats)} />
          )}
        </main>
      </div>

      {detailsId !== null && d.rows.some((r) => r.id === detailsId) && (
        <DetailsDialog api={api} row={d.rows.find((r) => r.id === detailsId)!} onClose={() => setDetailsId(null)} />
      )}
      {categories && <CategoriesDialog api={api} onClose={() => setCategories(false)} />}
      {queuesOpen && <QueuesDialog api={api} onClose={() => setQueuesOpen(false)} />}
      {rulesOpen && <RulesDialog api={api} queues={queueList} onClose={() => setRulesOpen(false)} />}
      {profilesOpen && <ProfilesDialog api={api} onClose={() => setProfilesOpen(false)} />}
      {schedule && <ScheduleDialog api={api} onClose={() => setSchedule(false)} />}
      {settings && <SettingsDialog api={api} clipboard={watchClipboard} onClipboard={setWatchClipboard} onClose={() => setSettings(false)} />}
      {adding && <AddDialog onAdd={addAll} queues={queueList} onBrowse={() => api.pickFolder()} defaultFolder={defaultFolder} onClose={() => setAdding(false)} />}
      {removing && (
        <Modal title={t("Remove from the list")} onClose={() => setRemoving(false)}>
          <p className="mt-0">
            {chosen.length === 1 ? t("Remove “{name}” from the list?", { name: chosen[0]!.name }) : t("Remove {count} downloads from the list?", { count: chosen.length })}
          </p>
          <label className="mb-4 flex items-center gap-2">
            <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
            {chosen.length === 1 ? t("Also delete the downloaded file from the disk") : t("Also delete the downloaded files from the disk")}
          </label>
          <div className="flex justify-end gap-2">
            <button className="btn" onClick={() => setRemoving(false)}>
              {t("Keep")}
            </button>
            <button
              className="btn btn-primary"
              onClick={async () => {
                const ok = await each(chosen, (id) => api.remove(id, deleteFiles));
                if (ok) setSelected(new Set());
                setRemoving(false);
              }}
            >
              {t("Remove")}
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
