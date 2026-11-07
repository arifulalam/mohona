import { useEffect, useRef, useState } from "react";
import { useT } from "./i18n";
import { linksIn } from "./droptarget";
import { linkIn } from "./clipboard";
import type { Api } from "./types";

type State = "idle" | "over" | "added" | "refused" | "failed";

/** The small always-on-top window: drop a link (or several) on it and it is downloaded. */
export function DropTarget({ api }: { api: Api }) {
  const t = useT();
  const [state, setState] = useState<State>("idle");
  const [count, setCount] = useState(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  function settle(next: State) {
    setState(next);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setState("idle"), 2500);
  }

  async function drop(event: React.DragEvent) {
    event.preventDefault();
    const data = event.dataTransfer;
    // A link dragged from a browser comes as a uri-list; selected text as plain text.
    const links = linksIn(data.getData("text/uri-list"), linkIn);
    const found = links.length ? links : linksIn(data.getData("text/plain"), linkIn);
    if (!found.length) return settle("refused");
    try {
      for (const link of found) await api.add(link, null);
      setCount(found.length);
      settle("added");
    } catch {
      settle("failed");
    }
  }

  const text =
    state === "over"
      ? t("Drop it")
      : state === "added"
        ? count === 1
          ? t("Added")
          : t("Added {n}", { n: count })
        : state === "refused"
          ? t("Not a link")
          : state === "failed"
            ? t("Not added")
            : t("Drop a link here");

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={t("Drop a link here")}
      title={t("Drop a link here. Click to open Mohona.")}
      onDragOver={(e) => {
        e.preventDefault();
        if (state !== "over") setState("over");
      }}
      onDragLeave={() => setState("idle")}
      onDrop={(e) => void drop(e)}
      onClick={() => void api.showMain().catch(() => undefined)}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && void api.showMain().catch(() => undefined)}
      className="flex h-screen w-screen cursor-pointer select-none flex-col items-center justify-center gap-1 border-2 border-dashed text-center text-[11px] leading-tight"
      style={{
        background: "var(--panel, var(--bg))",
        color: state === "refused" || state === "failed" ? "var(--bad)" : "var(--fg, inherit)",
        borderColor: state === "over" ? "var(--accent, currentColor)" : "var(--line)",
      }}
    >
      <span aria-hidden className="text-2xl">
        ⬇
      </span>
      <span role="status">{text}</span>
    </div>
  );
}
