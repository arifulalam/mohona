import { useEffect, useRef, useState } from "react";
import { useT } from "./i18n";
import type { Api, Row } from "./types";

interface Notice {
  id: number;
  name: string;
}

/** How long a message stays before it goes by itself. */
const STAY_MS = 15_000;
const MAX = 3;

/**
 * Downloads that finish while the window is open, each with a message that has Open and
 * "Show in folder" buttons. The first list seen is only a starting point, and the message is
 * shown only while "Show a message when a download finishes or fails" is on.
 */
export function useFinished(api: Api, rows: Row[]) {
  const [notices, setNotices] = useState<Notice[]>([]);
  const before = useRef<Map<number, string> | null>(null);

  useEffect(() => {
    const now = new Map(rows.map((r) => [r.id, r.status]));
    const was = before.current;
    before.current = now;
    if (was === null) return;
    const fresh = rows.filter((r) => r.status === "completed" && was.has(r.id) && was.get(r.id) !== "completed");
    if (fresh.length === 0) return;
    let alive = true;
    api
      .config()
      .then((c) => c.notifications)
      .catch(() => true)
      .then((wanted) => {
        if (!alive || !wanted) return;
        setNotices((list) => [...list, ...fresh.map((r) => ({ id: r.id, name: r.name }))].slice(-MAX));
      });
    return () => {
      alive = false;
    };
  }, [rows, api]);

  return { notices, dismiss: (id: number) => setNotices((list) => list.filter((n) => n.id !== id)) };
}

export function FinishedMessages({
  notices,
  onOpen,
  onShow,
  onDismiss,
}: {
  notices: Notice[];
  onOpen(id: number): void;
  onShow(id: number): void;
  onDismiss(id: number): void;
}) {
  const t = useT();
  return (
    <div role="status" aria-live="polite" className="pointer-events-none fixed bottom-4 end-4 z-20 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2">
      {notices.map((n) => (
        <Message key={n.id} notice={n} onOpen={onOpen} onShow={onShow} onDismiss={onDismiss} t={t} />
      ))}
    </div>
  );
}

function Message({
  notice,
  onOpen,
  onShow,
  onDismiss,
  t,
}: {
  notice: Notice;
  onOpen(id: number): void;
  onShow(id: number): void;
  onDismiss(id: number): void;
  t: ReturnType<typeof useT>;
}) {
  useEffect(() => {
    const timer = setTimeout(() => onDismiss(notice.id), STAY_MS);
    return () => clearTimeout(timer);
  }, [notice.id, onDismiss]);
  return (
    <div className="pointer-events-auto rounded-lg border border-[var(--line)] bg-[var(--panel)] p-3 shadow-lg">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-xs text-[var(--muted)]">{t("Download finished")}</div>
          <div className="truncate font-medium" title={notice.name}>
            {notice.name}
          </div>
        </div>
        <button type="button" aria-label={t("Close")} onClick={() => onDismiss(notice.id)}>
          ×
        </button>
      </div>
      <div className="mt-2 flex gap-2">
        <button
          type="button"
          onClick={() => {
            onOpen(notice.id);
            onDismiss(notice.id);
          }}
        >
          {t("Open")}
        </button>
        <button
          type="button"
          onClick={() => {
            onShow(notice.id);
            onDismiss(notice.id);
          }}
        >
          {t("Show in folder")}
        </button>
      </div>
    </div>
  );
}
