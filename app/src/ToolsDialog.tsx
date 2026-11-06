import { useEffect, useState } from "react";
import { useT } from "./i18n";
import { Modal } from "./AddDialog";
import type { Api, ToolInfo } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

/** ffmpeg and yt-dlp: what each is for, whether it is found, where to get it, and where it is. */
export function ToolsDialog({ api, onClose }: { api: Api; onClose(): void }) {
  const t = useT();
  const [tools, setTools] = useState<ToolInfo[] | null>(null);
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api
      .tools()
      .then((list) => {
        if (!alive) return;
        setTools(list);
        setPaths(Object.fromEntries(list.map((x) => [x.name, x.chosen ?? ""])));
      })
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [api]);

  async function choose(name: string, path: string | null) {
    setBusy(true);
    try {
      const list = await api.setTool(name, path);
      setTools(list);
      setPaths(Object.fromEntries(list.map((x) => [x.name, x.chosen ?? ""])));
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={t("Tools")} onClose={onClose}>
      <p className="mt-0 text-sm text-[var(--muted)]">
        {t("Mohona uses these two programs for some jobs. They are not part of Mohona: install them once, and Mohona finds them by itself, or tell it where they are.")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-[var(--bad)]">
          {error}
        </p>
      )}
      {tools === null && !error && <p>{t("Loading…")}</p>}
      <div className="flex flex-col gap-3">
        {tools?.map((tool) => (
          <section key={tool.name} aria-label={tool.name} className="rounded-lg border border-[var(--line)] p-3">
            <div className="flex items-center gap-2">
              <strong>{tool.name}</strong>
              <span className="text-sm" style={{ color: tool.found ? "var(--ok)" : "var(--bad)" }}>
                {tool.found ? t("Found") : t("Not found")}
              </span>
            </div>
            <p className="my-1 text-sm text-[var(--muted)]">
              {t("Needed to:")} {tool.purpose}
            </p>
            {tool.found && (
              <p className="my-1 break-all text-sm">
                {t("Using:")} {tool.found} {tool.chosen ? t("(your choice)") : t("(found by itself)")}
              </p>
            )}
            {!tool.found && tool.chosen && (
              <p role="alert" className="my-1 break-all text-sm text-[var(--bad)]">
                {t("Not at the place you chose:")} {tool.chosen}
              </p>
            )}
            {!tool.found && (
              <div className="my-2 text-sm">
                <div>
                  {t("Install on Windows:")} <code className="select-all rounded bg-[var(--bg)] px-1">{tool.winget}</code>
                </div>
                <div className="mt-1 flex flex-wrap gap-2">
                  <button type="button" onClick={() => void api.openToolPage(tool.windowsPage).catch((e: Error) => setError(e.message))}>
                    {t("Download for Windows")}
                  </button>
                  <button type="button" onClick={() => void api.openToolPage(tool.page).catch((e: Error) => setError(e.message))}>
                    {t("Other systems and help")}
                  </button>
                </div>
              </div>
            )}
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <input
                className={`${field} min-w-0 flex-1`}
                aria-label={`${tool.name}: ${t("Where it is")}`}
                placeholder={t("Where it is (the file, or its folder)")}
                value={paths[tool.name] ?? ""}
                onChange={(e) => setPaths({ ...paths, [tool.name]: e.target.value })}
              />
              <button type="button" disabled={busy || !(paths[tool.name] ?? "").trim()} onClick={() => void choose(tool.name, (paths[tool.name] ?? "").trim())}>
                {t("Use this")}
              </button>
              <button type="button" disabled={busy || !tool.chosen} onClick={() => void choose(tool.name, null)}>
                {t("Find it by itself")}
              </button>
            </div>
          </section>
        ))}
      </div>
    </Modal>
  );
}
