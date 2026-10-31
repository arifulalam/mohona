import { useEffect, useState } from "react";
import { tr, useT } from "./i18n";
import { Modal } from "./AddDialog";
import * as fmt from "./format";
import type { Api, Details, Row } from "./types";

const STATE: Record<string, string> = { active: tr("Receiving"), waiting: tr("Waiting"), done: tr("Done") };

/** One bar for the whole file with each connection's part drawn where it belongs. */
export function PartsMap({ details }: { details: Details }) {
  const t = useT();
  const size = details.size ?? 0;
  if (!size || details.connections.length === 0) return null;
  return (
    <div className="relative h-4 overflow-hidden rounded bg-[var(--line)]" role="img" aria-label={t("Which parts of the file are received")}>
      {details.connections.map((c) => (
        <span
          key={c.number}
          className="absolute top-0 h-full"
          style={{
            left: `${(c.start / size) * 100}%`,
            width: `${(c.received / size) * 100}%`,
            background: c.state === "done" ? "var(--ok)" : "var(--accent)",
            opacity: c.state === "waiting" ? 0.5 : 1,
            borderRight: "1px solid var(--panel)",
          }}
        />
      ))}
    </div>
  );
}

export function DetailsDialog({ api, row, onClose }: { api: Api; row: Row; onClose(): void }) {
  const t = useT();
  const [details, setDetails] = useState<Details | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .details(row.id)
        .then((d) => {
          if (alive) {
            setDetails(d);
            setError(null);
          }
        })
        .catch((e: Error) => alive && setError(e.message));
    void load();
    const timer = setInterval(load, 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [api, row.id]);

  const size = details?.size ?? row.size;
  const downloaded = details?.downloaded ?? row.downloaded;
  const percent = size ? Math.min(100, (downloaded / size) * 100) : null;
  const total = details?.speed ?? row.speed;
  const left = size && total > 0 ? Math.ceil((size - downloaded) / total) : null;
  const connections = details?.connections ?? [];
  const active = connections.filter((c) => c.state === "active").length;

  return (
    <Modal title={t("Download details")} onClose={onClose}>
      <div className="flex flex-col gap-3">
        <div>
          <div className="truncate font-medium" title={row.url}>
            {row.name}
          </div>
          <div className="truncate text-[12px] muted" title={row.url}>
            {row.url}
          </div>
        </div>
        <div className="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent ?? 0)} aria-label={t("Progress")}>
          <span style={{ width: `${percent ?? 0}%` }} />
        </div>
        <dl className="m-0 grid grid-cols-2 gap-x-6 gap-y-1 sm:grid-cols-4">
          {[
            ["Received", size ? t("{done} of {total}", { done: fmt.bytes(downloaded), total: fmt.bytes(size) }) : fmt.bytes(downloaded)],
            ["Progress", percent === null ? t("Size unknown") : `${percent.toFixed(1)}%`],
            ["Speed", total > 0 ? fmt.speed(total) : "—"],
            ["Time left", left !== null ? fmt.eta(left) : "—"],
          ].map(([name, value]) => (
            <div key={name}>
              <dt className="text-[11px] uppercase tracking-wide muted">{t(name!)}</dt>
              <dd className="m-0 font-medium">{value}</dd>
            </div>
          ))}
        </dl>

        {details && details.connections.length > 0 && <PartsMap details={details} />}

        <div>
          <h3 className="mb-1 mt-0 text-[12px] uppercase tracking-wide muted">
            {t("Connections")}
            {connections.length > 0 ? ` (${t("{active} receiving of {total}", { active, total: connections.length })})` : ""}
          </h3>
          {details === null ? (
            <p className={error ? "text-[var(--bad)]" : "muted"} role={error ? "alert" : "status"}>
              {error ?? t("Loading…")}
            </p>
          ) : connections.length === 0 ? (
            <p className="m-0 muted">
              {["downloading", "probing", "retrying"].includes(details.status)
                ? t("One connection, or the connections are still being set up.")
                : t("The connections show while the download is running.")}
            </p>
          ) : (
            <table className="w-full border-collapse text-[12px]">
              <thead>
                <tr className="text-left muted">
                  <th className="py-1 pr-2 font-normal">#</th>
                  <th className="py-1 pr-2 font-normal">{t("Part of the file")}</th>
                  <th className="py-1 pr-2 font-normal">{t("Received")}</th>
                  <th className="py-1 pr-2 text-right font-normal">{t("Speed")}</th>
                  <th className="py-1 text-right font-normal">{t("State")}</th>
                </tr>
              </thead>
              <tbody>
                {connections.map((c) => {
                  const part = c.end - c.start;
                  const pct = part > 0 ? (c.received / part) * 100 : 100;
                  return (
                    <tr key={c.number} className="border-t border-[var(--line)]">
                      <td className="py-1 pr-2">{c.number}</td>
                      <td className="py-1 pr-2 muted">
                        {fmt.bytes(c.start)} – {fmt.bytes(c.end)}
                      </td>
                      <td className="w-32 py-1 pr-2">
                        <div className={`bar ${c.state === "done" ? "done" : c.state === "waiting" ? "idle" : ""}`} aria-label={`Connection ${c.number} progress`}>
                          <span style={{ width: `${pct}%` }} />
                        </div>
                      </td>
                      <td className="py-1 pr-2 text-right">{c.speed > 0 ? fmt.speed(c.speed) : "—"}</td>
                      <td className="py-1 text-right">{STATE[c.state] ? t(STATE[c.state]!) : c.state}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
        <div className="flex justify-end">
          <button className="btn" onClick={onClose}>
            {t("Close")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
