import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import { ACTIONS } from "./schedule";
import { WindowsEditor } from "./WindowsEditor";
import type { Api, Plan, PlanWindow } from "./types";

export function ScheduleDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const t = useT();
  const [plan, setPlan] = useState<Plan | null>(null);
  const [windows, setWindows] = useState<PlanWindow[]>([]);
  const [action, setAction] = useState("");
  const [seconds, setSeconds] = useState("60");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api
      .plan()
      .then((p) => {
        if (!alive) return;
        setPlan(p);
        setWindows(p.windows);
        setAction(p.action ?? "");
      })
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    const countdown = Number(seconds);
    if (action && (!Number.isInteger(countdown) || countdown < 5 || countdown > 3600)) {
      return setError(t("The countdown must be a whole number of seconds from 5 to 3600."));
    }
    setBusy(true);
    try {
      await api.setPlan(windows);
      if (action !== (plan?.action ?? "")) await api.setAfterAction(action || null, action ? countdown : undefined);
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";
  return (
    <Modal title={t("Schedule")} onClose={onClose}>
      {!plan ? (
        <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
          {error ?? t("Loading…")}
        </p>
      ) : (
        <form onSubmit={save} className="flex flex-col gap-4">
          <div className="flex max-h-[60vh] flex-col gap-4 overflow-auto pr-1">
            <WindowsEditor windows={windows} onChange={setWindows} onError={setError} />

            <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
              <legend className="mb-1 font-semibold">{t("When all downloads are done")}</legend>
              <select className={field} value={action} onChange={(e) => setAction(e.target.value)} aria-label={t("When everything is done")}>
                {ACTIONS.map((a) => (
                  <option key={a.id} value={a.id}>
                    {t(a.label)}
                  </option>
                ))}
              </select>
              {action && (
                <label className="flex items-center gap-2">
                  {t("Wait")}
                  <input className={`${field} w-20`} value={seconds} onChange={(e) => setSeconds(e.target.value)} aria-label={t("Countdown seconds")} />
                  {t("seconds first (you can cancel)")}
                </label>
              )}
              <small className="muted">{t("It happens once, only for downloads that finish after you save this.")}</small>
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
