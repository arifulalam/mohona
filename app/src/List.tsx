import * as fmt from "./format";
import { tr, useT, type Translate } from "./i18n";
import type { Row } from "./types";

const LABEL: Record<string, string> = {
  queued: tr("Waiting"),
  probing: tr("Connecting"),
  downloading: tr("Downloading"),
  paused: tr("Paused"),
  retrying: tr("Retrying"),
  completed: tr("Finished"),
  failed: tr("Failed"),
  canceled: tr("Cancelled"),
};

function barClass(row: Row): string {
  if (row.status === "completed") return "done";
  if (row.status === "failed") return "bad";
  if (row.status === "paused" || row.status === "canceled" || row.status === "queued") return "idle";
  return "";
}

function detail(row: Row, t: Translate): string {
  if (row.status === "failed") {
    return row.expired ? t("The link has expired. Add the file again from its page.") : (row.error ?? t("Failed"));
  }
  if (row.status === "completed" && row.error) return `⚠ ${row.error}`;
  const done = fmt.bytes(row.downloaded);
  return row.size ? t("{done} of {total}", { done, total: fmt.bytes(row.size) }) : done;
}

function Icon({ d }: { d: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

interface Props {
  rows: Row[];
  selected: Set<number>;
  onSelect(id: number, additive: boolean): void;
  onPause(id: number): void;
  onResume(id: number): void;
  onOpen(id: number): void;
  onShow(id: number): void;
  onDetails(id: number): void;
  /** The category of a row, shown as a small label. */
  category(row: Row): string;
}

export function List({ rows, selected, onSelect, onPause, onResume, onOpen, onShow, onDetails, category }: Props) {
  const t = useT();
  return (
    <ul role="listbox" aria-multiselectable="true" aria-label={t("Downloads")} className="m-0 list-none p-0">
      {rows.map((row) => {
        const isSelected = selected.has(row.id);
        const running = ["queued", "probing", "downloading", "retrying"].includes(row.status);
        const stopped = ["paused", "failed", "canceled"].includes(row.status);
        return (
          <li
            key={row.id}
            role="option"
            aria-selected={isSelected}
            tabIndex={0}
            data-status={row.status}
            onClick={(e) => onSelect(row.id, e.ctrlKey || e.metaKey)}
            onDoubleClick={() => (running ? onDetails(row.id) : row.status === "completed" ? onOpen(row.id) : undefined)}
            onKeyDown={(e) => {
              if (e.key === " " || e.key === "Enter") {
                e.preventDefault();
                onSelect(row.id, e.ctrlKey || e.metaKey);
              }
            }}
            className="grid cursor-default grid-cols-[1fr_auto] items-center gap-x-4 gap-y-1 border-b border-[var(--line)] px-4 py-3 hover:bg-[var(--hover)] aria-selected:bg-[var(--selected)]"
          >
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="truncate font-medium" title={row.url}>
                  {row.name}
                </span>
                <span className="chip shrink-0" title={t("Category")}>
                  {t(category(row))}
                </span>
              </div>
              <div className="truncate text-[12px] muted">{detail(row, t)}</div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-28 text-right text-[12px] muted">
                <div>{fmt.speed(row.speed)}</div>
                <div>{row.eta !== null ? t("{time} left", { time: fmt.eta(row.eta) }) : ""}</div>
              </div>
              <span className="w-24 text-right text-[12px]" style={{ color: row.status === "failed" ? "var(--bad)" : row.status === "completed" ? "var(--ok)" : undefined }}>
                {LABEL[row.status] ? t(LABEL[row.status]!) : row.status}
              </span>
              {running && (
                <button className="btn !px-2 !py-0.5 text-[12px]" aria-label={t("Details of {name}", { name: row.name })} title={t("Connections, speed and parts")} onClick={(e) => (e.stopPropagation(), onDetails(row.id))}>
                  {t("Details")}
                </button>
              )}
              {row.status === "completed" && (
                <span className="flex gap-1">
                  <button className="btn !px-2 !py-0.5 text-[12px]" aria-label={t("Open {name}", { name: row.name })} onClick={(e) => (e.stopPropagation(), onOpen(row.id))}>
                    {t("Open")}
                  </button>
                  <button className="btn !px-2 !py-0.5 text-[12px]" aria-label={t("Show {name} in its folder", { name: row.name })} onClick={(e) => (e.stopPropagation(), onShow(row.id))}>
                    {t("Show in folder")}
                  </button>
                </span>
              )}
              <span className="w-8">
                {running && (
                  <button className="btn !p-1" aria-label={t("Pause {name}", { name: row.name })} title={t("Pause")} onClick={(e) => (e.stopPropagation(), onPause(row.id))}>
                    <Icon d="M6 5h4v14H6zM14 5h4v14h-4z" />
                  </button>
                )}
                {stopped && (
                  <button className="btn !p-1" aria-label={t("Resume {name}", { name: row.name })} title={t("Resume")} onClick={(e) => (e.stopPropagation(), onResume(row.id))}>
                    <Icon d="M8 5v14l11-7z" />
                  </button>
                )}
              </span>
            </div>
            <div className={`bar col-span-2 ${barClass(row)}`} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(row.percent ?? 0)} aria-label={t("{name} progress", { name: row.name })}>
              <span style={{ width: `${row.percent ?? (row.status === "completed" ? 100 : 0)}%` }} />
            </div>
          </li>
        );
      })}
    </ul>
  );
}
