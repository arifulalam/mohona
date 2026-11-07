import { useEffect, useMemo } from "react";
import { DropTarget } from "./DropTarget";
import { I18n, LANGUAGES, loadLanguage, translator } from "./i18n";
import type { Api } from "./types";

/** The drop target's own window: the language the person chose, and nothing else. */
export function DropWindow({ api }: { api: Api }) {
  const lang = useMemo(() => {
    const code = loadLanguage().code;
    return LANGUAGES.find((l) => l.code === code) ?? LANGUAGES[0]!;
  }, []);
  const t = useMemo(() => translator(lang.dict), [lang]);
  useEffect(() => {
    document.documentElement.lang = lang.code;
    document.documentElement.dir = lang.dir;
  }, [lang]);
  return (
    <I18n.Provider value={t}>
      <DropTarget api={api} />
    </I18n.Provider>
  );
}
