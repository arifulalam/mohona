// Makes the colour themes of the app and the extension from design/themes.json.
// Run: node design/themes.mjs        (add --check to only test the contrast)
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const { keys, themes } = JSON.parse(readFileSync(join(here, "themes.json"), "utf8"));

function luminance(hex) {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
const contrast = (a, b) => {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

// Text must be readable on every surface, buttons on their own colour.
const pairs = [
  ["text", "bg", 7], ["text", "panel", 7], ["muted", "bg", 4.5], ["muted", "panel", 4.5],
  ["accent-text", "accent", 4.5], ["accent", "panel", 3], ["ok", "panel", 4.5], ["bad", "panel", 4.5], ["warn", "panel", 4.5],
  ["text", "selected", 4.5], ["text", "hover", 7],
];
let failed = false;
for (const t of themes) {
  for (const [fg, bg, min] of pairs) {
    const c = contrast(t[fg], t[bg]);
    if (c < min) {
      failed = true;
      console.error(`${t.id}: ${fg} on ${bg} is ${c.toFixed(2)}:1, needs ${min}:1`);
    }
  }
}
for (const t of themes) {
  for (const stop of t.hero.match(/#[0-9a-fA-F]{6}/g) ?? []) {
    const c = contrast(t.heroText, stop);
    if (c < 4) {
      failed = true;
      console.error(`${t.id}: header text on ${stop} is ${c.toFixed(2)}:1, needs 4:1`);
    }
  }
}
if (failed) process.exit(1);
if (process.argv.includes("--check")) process.exit(0);

const css = themes
  .map((t) => {
    const lines = keys.map((k) => `  --${k}: ${t[k]};`);
    return `:root[data-theme="${t.id}"] {\n${lines.join("\n")}\n  --hero: ${t.hero};\n  --hero-text: ${t.heroText};\n  color-scheme: ${t.dark ? "dark" : "light"};\n}\n`;
  })
  .join("\n");
const header = "/* Made by design/themes.mjs from design/themes.json. Do not edit by hand. */\n";
const list = themes.map((t) => ({ id: t.id, label: t.label, dark: t.dark, bg: t.bg, accent: t.accent, hero: t.hero }));
const ts = `// Made by design/themes.mjs from design/themes.json. Do not edit by hand.\nexport const NAMED_THEMES = ${JSON.stringify(list, null, 2)} as const;\n`;
for (const [folder, cssName] of [["../app/src", "themes.css"], ["../extension/static", "themes.css"]]) {
  writeFileSync(join(here, folder, cssName), header + css);
}
writeFileSync(join(here, "../app/src/themes.generated.ts"), ts);
writeFileSync(join(here, "../extension/src/themes.generated.ts"), ts);
console.log(`${themes.length} themes written`);
