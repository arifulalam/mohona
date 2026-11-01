// Lists the popup's texts and starts a new language file.
//
//   node scripts/i18n.mjs            writes src/locales/_template.json (every text, empty)
//   node scripts/i18n.mjs --new xx   starts src/locales/xx.json for a new language (code "xx")
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export function popupTexts() {
  const found = new Set();
  const ts = readFileSync(join(root, "src", "popup.ts"), "utf8");
  for (const m of ts.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) found.add(JSON.parse(`"${m[1]}"`));
  const html = readFileSync(join(root, "static", "popup.html"), "utf8");
  for (const m of html.matchAll(/data-t(?:-label|-placeholder)?="([^"]+)"/g)) found.add(m[1]);
  const table = ts.match(/const LABEL[^{]*\{([^}]*)\}/)?.[1] ?? "";
  for (const m of table.matchAll(/:\s*"([^"]+)"/g)) found.add(m[1]);
  for (const m of readFileSync(join(root, "src", "themes.generated.ts"), "utf8").matchAll(/"label": "([^"]+)"/g)) found.add(m[1]);
  for (const name of ["System", "Light", "Dark"]) found.add(name);
  return [...found].sort((a, b) => a.localeCompare(b));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const texts = popupTexts();
  const template = {
    _name: "Your language's name, as its speakers write it",
    _english: "The same name in English",
    _dir: "ltr",
    ...Object.fromEntries(texts.map((t) => [t, ""])),
  };
  const body = JSON.stringify(template, null, 2) + "\n";
  writeFileSync(join(root, "src", "locales", "_template.json"), body);
  console.log(`${texts.length} texts; template written to src/locales/_template.json`);
  const at = process.argv.indexOf("--new");
  if (at > 0) {
    const code = process.argv[at + 1];
    if (!code || !/^[a-z]{2,3}(-[A-Za-z]{2,4})?$/.test(code)) {
      console.error('Give a language code such as "ne" or "pt-BR".');
      process.exit(1);
    }
    const file = join(root, "src", "locales", `${code}.json`);
    if (existsSync(file)) {
      console.error(`${code}.json exists already.`);
      process.exit(1);
    }
    writeFileSync(file, body);
    console.log(`Started src/locales/${code}.json: fill in the empty texts (empty ones show in English).`);
  }
}
