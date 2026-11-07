import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import type { Api, Seeding } from "./types";

const field = "w-24 rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

/** A number typed in a box: empty or 0 is "no limit" (null); anything else that is not a positive number is refused. */
function parse(text: string, max: number): number | null | "bad" {
  const clean = text.trim().replace(",", ".");
  if (clean === "") return null;
  const n = Number(clean);
  if (!Number.isFinite(n) || n < 0 || n > max) return "bad";
  return n === 0 ? null : n;
}

/** How long finished torrents are shared: until an upload ratio and/or a time, or not at all. */
export function TorrentsDialog({ api }: { api: Api }) {
  const t = useT();
  const [ratio, setRatio] = useState("");
  const [minutes, setMinutes] = useState("");
  const [saved, setSaved] = useState<Seeding | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function show(s: Seeding) {
    setSaved(s);
    setRatio(s.ratio ? String(s.ratio / 100) : "");
    setMinutes(s.minutes ? String(s.minutes) : "");
  }

  useEffect(() => {
    let alive = true;
    api
      .seeding()
      .then((s) => alive && show(s))
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function save(next: { ratio: number | null; minutes: number | null }) {
    setBusy(true);
    try {
      show(await api.setSeeding(next));
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  function apply() {
    const r = parse(ratio, 1000);
    const m = parse(minutes, 10_000_000);
    if (r === "bad" || m === "bad") {
      setError(t("Type a number (for example 1.5 for the ratio, 120 for the minutes), or leave a box empty."));
      return;
    }
    void save({ ratio: r === null ? null : Math.round(r * 100), minutes: m === null ? null : Math.round(m) });
  }

  const sharing = saved !== null && (saved.ratio !== null || saved.minutes !== null);

  return (
    <Modal title={t("Torrents")} onClose={() => undefined}>
      <p className="mt-0 text-sm text-[var(--muted)]">
        {t("While a torrent downloads, Mohona uploads to others as the protocol asks. After it finishes it can keep sharing (seeding) until one of these limits is reached. With both empty, a finished torrent is not shared.")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-[var(--bad)]">
          {error}
        </p>
      )}
      <div className="flex flex-col gap-2">
        <label className="flex flex-wrap items-center gap-2">
          {t("Stop sharing at an upload ratio of")}
          <input className={field} inputMode="decimal" value={ratio} onChange={(e) => setRatio(e.target.value)} placeholder="1.5" />
        </label>
        <label className="flex flex-wrap items-center gap-2">
          {t("or after (minutes)")}
          <input className={field} inputMode="numeric" value={minutes} onChange={(e) => setMinutes(e.target.value)} placeholder="120" />
        </label>
        <p className="my-0 text-xs text-[var(--muted)]">{t("Whichever comes first. The files stay in your download folder.")}</p>
        <p className="my-0 text-sm" role="status">
          {saved === null ? t("Loading…") : sharing ? t("Finished torrents are shared.") : t("Finished torrents are not shared.")}
        </p>
        <div className="flex gap-2">
          <button type="button" disabled={busy} onClick={apply}>
            {t("Save")}
          </button>
          <button type="button" disabled={busy || !sharing} onClick={() => void save({ ratio: null, minutes: null })}>
            {t("Do not share")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
