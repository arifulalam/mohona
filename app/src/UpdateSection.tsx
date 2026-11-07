import { useEffect, useState } from "react";
import { useT } from "./i18n";
import type { Api, UpdateInfo } from "./types";

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

/** Looking for a newer Mohona on GitHub Releases. Mohona only looks; the person installs from the page. */
export function UpdateSection({ api }: { api: Api }) {
  const t = useT();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [repo, setRepo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .update()
      .then((x) => {
        if (!alive) return;
        setInfo(x);
        setRepo(x.repo);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [api]);

  async function run(job: () => Promise<UpdateInfo>) {
    setBusy(true);
    setError(null);
    try {
      const x = await job();
      setInfo(x);
      setRepo(x.repo);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (!info) return null;
  return (
    <section aria-label={t("Updates")} className="rounded-lg border border-[var(--line)] p-3">
      <strong>{t("Updates")}</strong>
      <p className="my-1 text-sm text-[var(--muted)]">
        {t("Mohona can look on GitHub for a newer version. It only looks: you install it yourself from the release page.")}
      </p>
      <p className="my-1 text-sm">{t("This is version {v}.", { v: info.current })}</p>
      <div className="flex flex-wrap items-center gap-2">
        <input
          className={`${field} min-w-0 flex-1`}
          aria-label={t("GitHub repository")}
          placeholder={t("owner/name")}
          value={repo}
          onChange={(e) => setRepo(e.target.value)}
        />
        <button type="button" disabled={busy || repo.trim() === info.repo} onClick={() => void run(() => api.setUpdateRepo(repo.trim()))}>
          {t("Save")}
        </button>
        <button type="button" disabled={busy || !info.repo || repo.trim() !== info.repo} onClick={() => void run(() => api.checkUpdate())}>
          {t("Check for updates")}
        </button>
      </div>
      {error && (
        <p role="alert" className="my-1 text-sm text-[var(--bad)]">
          {error}
        </p>
      )}
      {info.latest && !info.newer && (
        <p role="status" className="my-1 text-sm">
          {t("You have the newest version.")}
        </p>
      )}
      {info.latest && info.newer && (
        <div role="status" className="my-1 text-sm">
          <strong>{t("A newer version is out: {v}", { v: info.latest })}</strong>
          {info.url && (
            <div className="mt-1">
              <button type="button" onClick={() => void api.openReleasePage(info.url!).catch((e: Error) => setError(e.message))}>
                {t("Open the release page")}
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
