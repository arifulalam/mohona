import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import { parseLimit, splitLimit, type Unit } from "./limits";
import type { Api, SiteProfile } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

export function ProfilesDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const t = useT();
  const [list, setList] = useState<SiteProfile[] | null>(null);
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [connections, setConnections] = useState("");
  const [limit, setLimit] = useState("");
  const [unit, setUnit] = useState<Unit>("KB");
  const [agent, setAgent] = useState("");
  const [referrer, setReferrer] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api.profiles().then((p) => alive && setList(p)).catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function save(next: SiteProfile[]): Promise<boolean> {
    setBusy(true);
    try {
      setList(await api.setProfiles(next));
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
    const speed = parseLimit(limit, unit);
    const count = connections.trim() === "" ? null : Number(connections);
    if (!name.trim()) return setError(t("Give the profile a name."));
    if (!host.trim()) return setError(t("Give the site, such as example.com."));
    if (count !== null && (!Number.isInteger(count) || count < 1 || count > 32)) return setError(t("Connections must be a whole number from 1 to 32."));
    if (speed === null) return setError(t("The speed limit must be a number, or empty for no limit."));
    if (count === null && !speed && !agent.trim() && !referrer.trim()) return setError(t("Set at least one thing: connections, a speed limit, a user agent or a referrer."));
    const profile: SiteProfile = {
      id: 0,
      name: name.trim(),
      enabled: true,
      host: host.trim(),
      connections: count,
      speedLimit: speed || null,
      userAgent: agent.trim() || null,
      referrer: referrer.trim() || null,
      headers: [],
    };
    if (await save([...(list ?? []), profile])) {
      setName("");
      setHost("");
      setConnections("");
      setLimit("");
      setAgent("");
      setReferrer("");
    }
  }

  const move = (at: number, by: number) => {
    const next = [...(list ?? [])];
    [next[at], next[at + by]] = [next[at + by]!, next[at]!];
    void save(next);
  };

  function facts(p: SiteProfile): string[] {
    const out = [p.host, "→"];
    if (p.connections !== null) out.push(t("Up to {count} connections", { count: p.connections }));
    if (p.speedLimit) {
      const l = splitLimit(p.speedLimit);
      out.push(t("{speed} per download", { speed: `${l.text} ${l.unit === "KB" ? t("KB/s") : t("MB/s")}` }));
    }
    if (p.userAgent) out.push(t("User agent: {value}", { value: p.userAgent }));
    if (p.referrer) out.push(t("Referrer: {value}", { value: p.referrer }));
    return out;
  }

  return (
    <Modal title={t("Site profiles")} onClose={onClose}>
      {!list ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? t("Loading…")}
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="muted m-0">{t("A profile sets how downloads from one site are made. The first profile that matches the site is used.")}</p>
          <ul className="m-0 flex max-h-[30vh] list-none flex-col gap-1 overflow-auto p-0" aria-label={t("Site profiles")}>
            {list.length === 0 && <li className="muted">{t("No site profiles yet.")}</li>}
            {list.map((p, at) => (
              <li key={p.id} className="flex items-start gap-2 rounded-md border border-[var(--line)] px-2 py-1">
                <input type="checkbox" className="mt-1" checked={p.enabled} disabled={busy} aria-label={t("Use profile “{name}”", { name: p.name })} onChange={(e) => void save(list.map((x) => (x === p ? { ...x, enabled: e.target.checked } : x)))} />
                <span className="min-w-0 flex-1 break-words" style={p.enabled ? undefined : { opacity: 0.55 }}>
                  <b>{p.name}</b>
                  <br />
                  <span className="muted">{facts(p).join("  ")}</span>
                </span>
                <button type="button" className="btn !px-2" disabled={busy || at === 0} aria-label={t("Move up")} title={t("Move up")} onClick={() => move(at, -1)}>
                  ↑
                </button>
                <button type="button" className="btn !px-2" disabled={busy || at === list.length - 1} aria-label={t("Move down")} title={t("Move down")} onClick={() => move(at, 1)}>
                  ↓
                </button>
                <button type="button" className="btn btn-danger !px-2" disabled={busy} aria-label={t("Remove")} title={t("Remove")} onClick={() => void save(list.filter((x) => x !== p))}>
                  ✕
                </button>
              </li>
            ))}
          </ul>
          <form onSubmit={add} className="flex flex-col gap-2 border-t border-[var(--line)] pt-3">
            <b>{t("Add a site profile")}</b>
            <div className="flex gap-2">
              <input className={`${field} min-w-0 flex-1`} value={name} onChange={(e) => setName(e.target.value)} placeholder={t("Profile name")} aria-label={t("Profile name")} />
              <input className={`${field} min-w-0 flex-1`} value={host} onChange={(e) => setHost(e.target.value)} placeholder={t("Site (such as example.com)")} aria-label={t("Site (such as example.com)")} />
            </div>
            <div className="flex flex-wrap gap-2">
              <input className={`${field} w-36`} value={connections} onChange={(e) => setConnections(e.target.value)} placeholder={t("Most connections")} aria-label={t("Most connections")} />
              <span className="flex gap-1">
                <input className={`${field} w-40`} value={limit} onChange={(e) => setLimit(e.target.value)} placeholder={t("Speed limit per download")} aria-label={t("Speed limit per download")} />
                <select className={field} value={unit} onChange={(e) => setUnit(e.target.value as Unit)} aria-label={t("Speed unit")}>
                  <option value="KB">{t("KB/s")}</option>
                  <option value="MB">{t("MB/s")}</option>
                </select>
              </span>
            </div>
            <input className={field} value={agent} onChange={(e) => setAgent(e.target.value)} placeholder={t("User agent (optional)")} aria-label={t("User agent (optional)")} />
            <input className={field} value={referrer} onChange={(e) => setReferrer(e.target.value)} placeholder={t("Referrer (optional)")} aria-label={t("Referrer (optional)")} />
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
