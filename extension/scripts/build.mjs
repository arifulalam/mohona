// Builds dist/chromium and dist/firefox: two scripts (as classic, self-contained
// files, which every browser loads the same way), the popup and a manifest each.
import { cpSync, copyFileSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import { manifestFor } from "./manifests.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const dist = `${root}dist/`;
rmSync(dist, { recursive: true, force: true });

for (const target of ["chromium", "firefox"]) {
  const out = `${dist}${target}`;
  mkdirSync(out, { recursive: true });
  for (const name of ["background", "popup", "content"]) {
    await build({
      root,
      configFile: false,
      logLevel: "warn",
      build: {
        outDir: out,
        emptyOutDir: false,
        minify: false,
        target: "es2022",
        lib: { entry: `${root}src/${name}.ts`, formats: ["iife"], name: `mohona_${name}`, fileName: () => `${name}.js` },
      },
    });
  }
  cpSync(`${root}static/icons`, `${out}/icons`, { recursive: true });
  copyFileSync(`${root}static/popup.html`, `${out}/popup.html`);
  copyFileSync(`${root}static/themes.css`, `${out}/themes.css`);
  writeFileSync(`${out}/manifest.json`, JSON.stringify(manifestFor(target), null, 2) + "\n");
  console.log(`built ${target}`);
}
