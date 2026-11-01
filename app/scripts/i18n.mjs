// Finds every text the window shows and checks the language files against it.
//
//   node scripts/i18n.mjs            writes locales/_template.json (every text, empty)
//   node scripts/i18n.mjs --check    also reports what each language lacks or got wrong
//   node scripts/i18n.mjs --new xx   starts src/locales/xx.json for a new language (code "xx")
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "src");
const locales = join(src, "locales");

/** Every text in the source: t("..."), tr("...") and the names of the colour themes. */
export function findTexts() {
  const found = new Set();
  const call = /\b(?:t|tr)\(\s*"((?:[^"\\]|\\.)*)"/g;
  for (const name of readdirSync(src)) {
    if (!/\.(ts|tsx)$/.test(name) || name === "themes.generated.ts") continue;
    const text = readFileSync(join(src, name), "utf8");
    for (const m of text.matchAll(call)) found.add(JSON.parse(`"${m[1]}"`));
  }
  const themes = readFileSync(join(src, "themes.generated.ts"), "utf8");
  for (const m of themes.matchAll(/"label": "([^"]+)"/g)) found.add(m[1]);
  return [...found].sort((a, b) => a.localeCompare(b));
}

const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");

/** What is wrong with one language file: missing texts, texts the window no longer has, bad {names}. */
export function review(dict, texts) {
  const known = new Set(texts);
  const missing = texts.filter((t) => !dict[t]);
  const stale = Object.keys(dict).filter((k) => !k.startsWith("_") && !known.has(k));
  const broken = texts.filter((t) => dict[t] && placeholders(dict[t]) !== placeholders(t));
  return { missing, stale, broken };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const texts = findTexts();
  const template = { _name: "Your language's name, as its speakers write it", _english: "The same name in English", _dir: "ltr", ...Object.fromEntries(texts.map((t) => [t, ""])) };
  writeFileSync(join(locales, "_template.json"), JSON.stringify(template, null, 2) + "\n");
  console.log(`${texts.length} texts; template written to src/locales/_template.json`);

  const at = process.argv.indexOf("--new");
  if (at > 0) {
    const code = process.argv[at + 1];
    if (!code || !/^[a-z]{2,3}(-[A-Za-z]{2,4})?$/.test(code)) {
      console.error('Give a language code such as "ne" or "pt-BR".');
      process.exit(1);
    }
    const file = join(locales, `${code}.json`);
    if (existsSync(file)) {
      console.error(`${code}.json exists already.`);
      process.exit(1);
    }
    writeFileSync(file, JSON.stringify(template, null, 2) + "\n");
    console.log(`Started src/locales/${code}.json: fill in the empty texts (empty ones show in English).`);
  }

  if (process.argv.includes("--check")) {
    let bad = false;
    for (const name of readdirSync(locales).filter((n) => /^[a-z].*\.json$/.test(n))) {
      const dict = JSON.parse(readFileSync(join(locales, name), "utf8"));
      const { missing, stale, broken } = review(dict, texts);
      const done = texts.length - missing.length;
      console.log(`${name.padEnd(10)} ${done}/${texts.length} translated${stale.length ? `, ${stale.length} not used any more` : ""}${broken.length ? `, ${broken.length} with wrong {names}` : ""}`);
      for (const t of broken) console.log(`   wrong {names}: ${t}`);
      for (const t of stale) console.log(`   not used: ${t}`);
      if (broken.length || stale.length) bad = true;
    }
    if (bad) process.exit(1);
  }
}
