import { useEffect, useState } from "react";
import { Embedded } from "./AddDialog";
import { CategoriesDialog } from "./CategoriesDialog";
import { label, LANGUAGES, useT } from "./i18n";
import { ProfilesDialog } from "./ProfilesDialog";
import { QueuesDialog } from "./QueuesDialog";
import { RulesDialog } from "./RulesDialog";
import { TorrentsDialog } from "./TorrentsDialog";
import { UpdateSection } from "./UpdateSection";
import { ToolsDialog } from "./ToolsDialog";
import { ScheduleDialog } from "./ScheduleDialog";
import { SettingsDialog } from "./SettingsDialog";
import { THEMES, type Theme } from "./theme";
import type { Api } from "./types";

type Group = "general" | "downloads" | "categories" | "queues" | "schedule" | "rules" | "profiles" | "torrents" | "tools";

interface Props {
  api: Api;
  running: boolean;
  language: string;
  onLanguage(code: string): void;
  theme: Theme;
  onTheme(theme: Theme): void;
  clipboard: boolean;
  onClipboard(on: boolean): void;
  onClose(): void;
}

/** One window for everything that can be set: the groups on the left, the chosen group on the right. */
export function SettingsWindow(props: Props) {
  const { api, running, onClose } = props;
  const t = useT();
  const [group, setGroup] = useState<Group>("general");
  const [queues, setQueues] = useState<{ id: number; name: string }[]>([]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    if (running && group === "rules") api.queues().then(setQueues).catch(() => undefined);
  }, [api, running, group]);

  const groups: { id: Group; name: string; needsEngine: boolean }[] = [
    { id: "general", name: t("General"), needsEngine: false },
    { id: "downloads", name: t("Downloads"), needsEngine: true },
    { id: "categories", name: t("Categories"), needsEngine: true },
    { id: "queues", name: t("Queues"), needsEngine: true },
    { id: "schedule", name: t("Schedule"), needsEngine: true },
    { id: "rules", name: t("Rules"), needsEngine: true },
    { id: "profiles", name: t("Site profiles"), needsEngine: true },
    { id: "torrents", name: t("Torrents"), needsEngine: true },
    { id: "tools", name: t("Tools"), needsEngine: true },
  ];

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={t("Settings")} className="flex h-[min(640px,90vh)] w-full max-w-4xl overflow-hidden rounded-xl border border-[var(--line)] bg-[var(--panel)] shadow-xl">
        <nav aria-label={t("Settings groups")} className="flex w-48 shrink-0 flex-col gap-1 border-e border-[var(--line)] bg-[var(--bg)] p-3">
          <h2 className="m-0 mb-2 px-2 text-base font-semibold">{t("Settings")}</h2>
          {groups.map((g) => (
            <button
              key={g.id}
              type="button"
              aria-current={g.id === group ? "page" : undefined}
              disabled={g.needsEngine && !running}
              onClick={() => setGroup(g.id)}
              className="w-full rounded-md border-0 bg-transparent px-2 py-1.5 text-start hover:bg-[var(--hover)] disabled:opacity-50"
              style={g.id === group ? { background: "var(--selected)", fontWeight: 600 } : undefined}
            >
              {g.name}
            </button>
          ))}
        </nav>
        <section className="min-w-0 flex-1 overflow-auto p-5" aria-label={groups.find((g) => g.id === group)?.name}>
          <Embedded.Provider value={true}>
            <h2 className="mb-3 mt-0 text-base font-semibold">{groups.find((g) => g.id === group)?.name}</h2>
            {group === "general" && <General {...props} />}
            {group === "downloads" && <SettingsDialog api={api} clipboard={props.clipboard} onClipboard={props.onClipboard} onClose={onClose} />}
            {group === "categories" && <CategoriesDialog api={api} onClose={onClose} />}
            {group === "queues" && <QueuesDialog api={api} onClose={onClose} />}
            {group === "schedule" && <ScheduleDialog api={api} onClose={onClose} />}
            {group === "rules" && <RulesDialog api={api} queues={queues} onClose={onClose} />}
            {group === "profiles" && <ProfilesDialog api={api} onClose={onClose} />}
            {group === "torrents" && <TorrentsDialog api={api} />}
            {group === "tools" && <ToolsDialog api={api} onClose={onClose} />}
          </Embedded.Provider>
        </section>
      </div>
    </div>
  );
}

const field = "rounded-md border border-[var(--line)] bg-[var(--bg)] px-2 py-1";

function General({ api, running, language, onLanguage, theme, onTheme, onClose }: Props) {
  const t = useT();
  return (
    <div className="flex flex-col gap-4">
      <label className="flex flex-col gap-1">
        <span>{t("Language")}</span>
        <select className={`${field} max-w-xs`} value={language} onChange={(e) => onLanguage(e.target.value)} aria-label={t("Language")}>
          {LANGUAGES.map((l) => (
            <option key={l.code} value={l.code} lang={l.code}>
              {label(l)}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1">
        <span>{t("Colour theme")}</span>
        <select className={`${field} max-w-xs`} value={theme} onChange={(e) => onTheme(e.target.value as Theme)} aria-label={t("Colour theme")}>
          {THEMES.map((th) => (
            <option key={th.id} value={th.id}>
              {t(th.label)}
            </option>
          ))}
        </select>
      </label>
      {running && <UpdateSection api={api} />}
      <div className="flex justify-end">
        <button type="button" className="btn" onClick={onClose}>
          {t("Close")}
        </button>
      </div>
    </div>
  );
}
