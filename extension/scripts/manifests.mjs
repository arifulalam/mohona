import { readFileSync } from "node:fs";

const here = new URL("../", import.meta.url);
const read = (name) => JSON.parse(readFileSync(new URL(name, here), "utf8"));

/** The manifest for `target`: "chromium" (Chrome, Edge, Chromium) or "firefox". */
export function manifestFor(target) {
  const base = read("manifest.base.json");
  const identity = read("identity.json");
  if (target === "chromium") {
    return {
      ...base,
      // Pins the extension id (see identity.json), so the host can be registered for it.
      key: identity.chromiumKey,
      background: { service_worker: "background.js" },
    };
  }
  if (target === "firefox") {
    return {
      ...base,
      background: { scripts: ["background.js"] },
      browser_specific_settings: {
        gecko: { id: identity.firefoxId, strict_min_version: identity.firefoxMinVersion },
      },
    };
  }
  throw new Error(`unknown target ${target}`);
}
