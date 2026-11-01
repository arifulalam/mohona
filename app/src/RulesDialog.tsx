import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import { parseTypes } from "./CategoriesDialog";
import type { Api, Rule } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

export function RulesDialog({ api, queues, onClose }: { api: Api; queues: { id: number; name: string }[]; onClose(): void }) {
  const t = useT();
  const [list, setList] = useState<Rule[] | null>(null);
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [types, setTypes] = useState("");
  const [contains, setContains] = useState("");
  const [folder, setFolder] = useState("");
  const [queue, setQueue] = useState("");
  const [block, setBlock] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api.rules().then((r) => alive && setList(r)).catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function save(next: Rule[]): Promise<boolean> {
    setBusy(true);
    try {
      setList(await api.setRules(next));
      setError(null);
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
    const rule: Rule = {
      id: 0,
      name: name.trim(),
      enabled: true,
      host: host.trim() || null,
      extensions: parseTypes(types),
      urlContains: contains.trim() || null,
      folder: block ? null : folder.trim() || null,
      queue: block || !queue ? null : Number(queue),
      sort: null,
      block,
    };
    if (!rule.name) return setError(t("Give the rule a name."));
    if (!rule.host && rule.extensions.length === 0 && !rule.urlContains) return setError(t("Give at least one condition: a site, file types or text in the address."));
    if (!rule.block && !rule.folder && rule.queue === null) return setError(t("Choose what to do: a folder, a queue, or block."));
    if (await save([...(list ?? []), rule])) {
      setName("");
      setHost("");
      setTypes("");
      setContains("");
      setFolder("");
      setQueue("");
      setBlock(false);
    }
  }

  const move = (at: number, by: number) => {
    const next = [...(list ?? [])];
    [next[at], next[at + by]] = [next[at + by]!, next[at]!];
    void save(next);
  };

  function facts(rule: Rule): string[] {
    const out: string[] = [];
    if (rule.host) out.push(t("Site: {value}", { value: rule.host }));
    if (rule.extensions.length) out.push(t("Types: {value}", { value: rule.extensions.join(", ") }));
    if (rule.urlContains) out.push(t("Address has: {value}", { value: rule.urlContains }));
    out.push("→");
    if (rule.block) out.push(t("Blocked"));
    if (rule.folder) out.push(t("Folder: {value}", { value: rule.folder }));
    if (rule.queue !== null) out.push(t("Queue: {name}", { name: queues.find((q) => q.id === rule.queue)?.name ?? String(rule.queue) }));
    if (rule.sort) out.push(t("Sorted by category"));
    return out;
  }

  return (
    <Modal title={t("Rules")} onClose={onClose}>
      {!list ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? t("Loading…")}
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="muted m-0">{t("A rule sends matching downloads to a folder or queue, or blocks them. The first rule that matches is used.")}</p>
          <ul className="m-0 flex max-h-[32vh] list-none flex-col gap-1 overflow-auto p-0" aria-label={t("Rules")}>
            {list.length === 0 && <li className="muted">{t("No rules yet.")}</li>}
            {list.map((rule, at) => (
              <li key={rule.id} className="flex items-start gap-2 rounded-md border border-[var(--line)] px-2 py-1">
                <input type="checkbox" className="mt-1" checked={rule.enabled} disabled={busy} aria-label={t("Use rule “{name}”", { name: rule.name })} onChange={(e) => void save(list.map((r) => (r === rule ? { ...r, enabled: e.target.checked } : r)))} />
                <span className="min-w-0 flex-1 break-words" style={rule.enabled ? undefined : { opacity: 0.55 }}>
                  <b>{rule.name}</b>
                  <br />
                  <span className="muted">{facts(rule).join("  ")}</span>
                </span>
                <button type="button" className="btn !px-2" disabled={busy || at === 0} aria-label={t("Move up")} title={t("Move up")} onClick={() => move(at, -1)}>
                  ↑
                </button>
                <button type="button" className="btn !px-2" disabled={busy || at === list.length - 1} aria-label={t("Move down")} title={t("Move down")} onClick={() => move(at, 1)}>
                  ↓
                </button>
                <button type="button" className="btn btn-danger !px-2" disabled={busy} aria-label={t("Remove")} title={t("Remove")} onClick={() => void save(list.filter((r) => r !== rule))}>
                  ✕
                </button>
              </li>
            ))}
          </ul>
          <form onSubmit={add} className="flex flex-col gap-2 border-t border-[var(--line)] pt-3">
            <b>{t("Add a rule")}</b>
            <input className={field} value={name} onChange={(e) => setName(e.target.value)} placeholder={t("Rule name")} aria-label={t("Rule name")} />
            <div className="flex gap-2">
              <input className={`${field} min-w-0 flex-1`} value={host} onChange={(e) => setHost(e.target.value)} placeholder={t("Site (such as example.com)")} aria-label={t("Site (such as example.com)")} />
              <input className={`${field} min-w-0 flex-1`} value={types} onChange={(e) => setTypes(e.target.value)} placeholder={t("File types (such as zip, iso)")} aria-label={t("File types (such as zip, iso)")} />
            </div>
            <input className={field} value={contains} onChange={(e) => setContains(e.target.value)} placeholder={t("Address contains")} aria-label={t("Address contains")} />
            <label className="flex items-center gap-2">
              <input type="checkbox" checked={block} onChange={(e) => setBlock(e.target.checked)} /> {t("Block this download")}
            </label>
            {!block && (
              <div className="flex gap-2">
                <input className={`${field} min-w-0 flex-1`} value={folder} onChange={(e) => setFolder(e.target.value)} placeholder={t("Save in this folder (optional)")} aria-label={t("Save in this folder (optional)")} />
                <button type="button" className="btn" onClick={() => void api.pickFolder().then((f) => f && setFolder(f)).catch((e: Error) => setError(e.message))}>
                  {t("Browse…")}
                </button>
              </div>
            )}
            {!block && queues.length > 1 && (
              <label className="flex items-center gap-2">
                <span>{t("Put in queue")}</span>
                <select className={field} value={queue} onChange={(e) => setQueue(e.target.value)} aria-label={t("Put in queue")}>
                  <option value="">{t("(no change)")}</option>
                  {queues.map((q) => (
                    <option key={q.id} value={q.id}>
                      {q.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {error && (
              <p role="alert" className="m-0 text-[var(--bad)]">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <button type="button" className="btn" onClick={onClose}>
                {t("Close")}
              </button>
              <button type="submit" className="btn btn-primary" disabled={busy}>
                {t("Add")}
              </button>
            </div>
          </form>
        </div>
      )}
    </Modal>
  );
}
