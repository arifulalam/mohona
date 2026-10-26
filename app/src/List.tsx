import * as fmt from "./format";
import type { Row } from "./types";

const LABEL: Record<string, string> = {
  queued: "Waiting",
  probing: "Connecting",
  downloading: "Downloading",
  paused: "Paused",
  retrying: "Retrying",
  completed: "Finished",
  failed: "Failed",
  canceled: "Cancelled",
};

function barClass(row: Row): string {
  if (row.status === "completed") return "done";
  if (row.status === "failed") return "bad";
  if (row.status === "paused" || row.status === "canceled" || row.status === "queued") return "idle";
  return "";
}

function detail(row: Row): string {
  if (row.status === "failed") {
    return row.expired ? "The link has expired. Add the file again from its page." : (row.error ?? "Failed");
  }
  const done = fmt.bytes(row.downloaded);
  return row.size ? `${done} of ${fmt.bytes(row.size)}` : done;
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
}

export function List({ rows, selected, onSelect, onPause, onResume }: Props) {
  return (
    <ul role="listbox" aria-multiselectable="true" aria-label="Downloads" className="m-0 list-none p-0">
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
            onKeyDown={(e) => {
              if (e.key === " " || e.key === "Enter") {
                e.preventDefault();
                onSelect(row.id, e.ctrlKey || e.metaKey);
              }
            }}
            className="grid cursor-default grid-cols-[1fr_auto] items-center gap-x-4 gap-y-1 border-b border-[var(--line)] px-4 py-3 hover:bg-[var(--hover)] aria-selected:bg-[var(--selected)]"
          >
            <div className="min-w-0">
              <div className="truncate font-medium" title={row.url}>
                {row.name}
              </div>
              <div className="truncate text-[12px] muted">{detail(row)}</div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-28 text-right text-[12px] muted">
                <div>{fmt.speed(row.speed)}</div>
                <div>{row.eta !== null ? `${fmt.eta(row.eta)} left` : ""}</div>
              </div>
              <span className="w-24 text-right text-[12px]" style={{ color: row.status === "failed" ? "var(--bad)" : row.status === "completed" ? "var(--ok)" : undefined }}>
                {LABEL[row.status] ?? row.status}
              </span>
              <span className="w-8">
                {running && (
                  <button className="btn !p-1" aria-label={`Pause ${row.name}`} title="Pause" onClick={(e) => (e.stopPropagation(), onPause(row.id))}>
                    <Icon d="M6 5h4v14H6zM14 5h4v14h-4z" />
                  </button>
                )}
                {stopped && (
                  <button className="btn !p-1" aria-label={`Resume ${row.name}`} title="Resume" onClick={(e) => (e.stopPropagation(), onResume(row.id))}>
                    <Icon d="M8 5v14l11-7z" />
                  </button>
                )}
              </span>
            </div>
            <div className={`bar col-span-2 ${barClass(row)}`} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(row.percent ?? 0)} aria-label={`${row.name} progress`}>
              <span style={{ width: `${row.percent ?? (row.status === "completed" ? 100 : 0)}%` }} />
            </div>
          </li>
        );
      })}
    </ul>
  );
}
