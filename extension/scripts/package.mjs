// Makes the files to upload to the stores: dist/store/mohona-chromium-<version>.zip (Chrome Web
// Store, Edge Add-ons) and mohona-firefox-<version>.zip (Firefox Add-ons). The Chromium copy has no
// pinned `key` (the store signs it itself). Run `npm run build` first (`npm run package` does).
import { readdirSync, readFileSync, statSync, mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { zipSync } from "fflate";
import { manifestFor } from "./manifests.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const version = JSON.parse(readFileSync(`${root}manifest.base.json`, "utf8")).version;

function files(dir, prefix = "") {
  const out = {};
  for (const name of readdirSync(dir).sort()) {
    const path = `${dir}/${name}`;
    if (statSync(path).isDirectory()) Object.assign(out, files(path, `${prefix}${name}/`));
    else out[`${prefix}${name}`] = readFileSync(path);
  }
  return out;
}

mkdirSync(`${root}dist/store`, { recursive: true });
for (const target of ["chromium", "firefox"]) {
  const entries = files(`${root}dist/${target}`);
  entries["manifest.json"] = Buffer.from(JSON.stringify(manifestFor(target, { store: true }), null, 2) + "\n");
  const name = `${root}dist/store/mohona-${target}-${version}.zip`;
  // Fixed time stamps: the same sources give the same file.
  const zipped = zipSync(
    Object.fromEntries(Object.entries(entries).map(([k, v]) => [k, [v, { mtime: new Date(Date.UTC(2026, 0, 1)) }]])),
    { level: 9 },
  );
  writeFileSync(name, zipped);
  console.log(`${name} (${Object.keys(entries).length} files, ${zipped.length} bytes)`);
}
