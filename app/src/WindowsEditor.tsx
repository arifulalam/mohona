import { useState } from "react";
import { DAYS, DAY_LABEL, describeWindow, windowFrom } from "./schedule";
import { useT } from "./i18n";
import type { PlanWindow } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

/** The list of time windows of a queue, with a small form to add one. */
export function WindowsEditor({ windows, onChange, onError }: { windows: PlanWindow[]; onChange(w: PlanWindow[]): void; onError(message: string | null): void }) {
  const t = useT();
  const [days, setDays] = useState<string[]>([...DAYS]);
  const [start, setStart] = useState("23:00");
  const [end, setEnd] = useState("06:00");
  const [limit, setLimit] = useState("");

  function add() {
    const w = windowFrom(days, start, end, limit);
    if (typeof w === "string") return onError(t(w));
    onError(null);
    onChange([...windows, w]);
  }

  return (
    <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
              <legend className="mb-1 font-semibold">{t("When downloads may run")}</legend>
              <small className="muted">
                {windows.length === 0 ? t("No schedule: downloads run at any time.") : t("Downloads run only inside these windows and wait outside them.")}
              </small>
              <ul className="m-0 list-none p-0">
                {windows.map((w, i) => (
                  <li key={i} className="flex items-center justify-between gap-2 border-b border-[var(--line)] py-1">
                    <span>{describeWindow(w, t)}</span>
                    <button type="button" className="btn !px-2 !py-0.5" aria-label={t("Remove window {window}", { window: describeWindow(w, t) })} onClick={() => onChange(windows.filter((_, j) => j !== i))}>
                      {t("Remove")}
                    </button>
                  </li>
                ))}
              </ul>
              <div className="flex flex-wrap items-center gap-1" role="group" aria-label={t("Days")}>
                {DAYS.map((d) => (
                  <label key={d} className="flex items-center gap-1">
                    <input type="checkbox" checked={days.includes(d)} onChange={(e) => setDays(e.target.checked ? [...days, d] : days.filter((x) => x !== d))} />
                    {t(DAY_LABEL[d]!)}
                  </label>
                ))}
              </div>
              <div className="flex flex-wrap items-end gap-3">
                <label className="flex flex-col gap-1">
                  <span>{t("From")}</span>
                  <input className={`${field} w-20`} value={start} onChange={(e) => setStart(e.target.value)} aria-label={t("Window start")} />
                </label>
                <label className="flex flex-col gap-1">
                  <span>{t("To")}</span>
                  <input className={`${field} w-20`} value={end} onChange={(e) => setEnd(e.target.value)} aria-label={t("Window end")} />
                </label>
                <label className="flex flex-col gap-1">
                  <span>{t("Speed limit (KB/s, optional)")}</span>
                  <input className={`${field} w-28`} value={limit} onChange={(e) => setLimit(e.target.value)} aria-label={t("Window speed limit")} />
                </label>
                <button type="button" className="btn" onClick={add}>
                  {t("Add window")}
                </button>
              </div>
              <small className="muted">{t("A window can cross midnight (23:00 to 06:00).")}</small>
            </fieldset>
  );
}
