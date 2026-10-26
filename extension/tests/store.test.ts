import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { manifestFor } from "../scripts/manifests.mjs";

const store = readFileSync(new URL("../../docs/store.md", import.meta.url), "utf8");
const privacy = readFileSync(new URL("../../docs/privacy.md", import.meta.url), "utf8");

describe("store files", () => {
  it("explains every permission the extension asks for", () => {
    for (const target of ["chromium", "firefox"] as const) {
      const m = manifestFor(target) as any;
      for (const permission of m.permissions as string[]) {
        expect(store, permission).toContain(`\`${permission}\``);
      }
    }
  });

  it("leaves the pinned key out of the store copy, and keeps it for local builds", () => {
    expect((manifestFor("chromium", { store: true }) as any).key).toBeUndefined();
    expect((manifestFor("chromium") as any).key).toBeTypeOf("string");
  });

  it("has icons that are really in the build", () => {
    const m = manifestFor("chromium") as any;
    expect(Object.keys(m.icons)).toEqual(["16", "32", "48", "128"]);
    for (const file of Object.values(m.icons) as string[]) {
      readFileSync(new URL(`../static/${file}`, import.meta.url));
    }
  });

  it("tells Firefox that nothing is collected, in line with the policy", () => {
    expect((manifestFor("firefox") as any).browser_specific_settings.gecko.data_collection_permissions).toEqual({ required: ["none"] });
    expect(privacy).toContain("no servers");
  });
});
