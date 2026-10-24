import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { manifestFor } from "../scripts/manifests.mjs";

const identity = JSON.parse(readFileSync(new URL("../identity.json", import.meta.url), "utf8"));

describe("manifests", () => {
  it("ask only for what the extension uses (and nothing runs inside web pages)", () => {
    for (const target of ["chromium", "firefox"] as const) {
      const m = manifestFor(target) as any;
      expect(m.manifest_version).toBe(3);
      expect([...m.permissions].sort()).toEqual(["cookies", "downloads", "nativeMessaging", "storage"]);
      expect(m.host_permissions).toEqual(["<all_urls>"]);
      expect(m.content_scripts).toBeUndefined();
    }
  });

  it("use each browser's kind of background", () => {
    expect((manifestFor("chromium") as any).background).toEqual({ service_worker: "background.js" });
    const firefox = manifestFor("firefox") as any;
    expect(firefox.background).toEqual({ scripts: ["background.js"] });
    expect(firefox.browser_specific_settings.gecko.id).toBe("mohona@mohona.local");
    expect(firefox.key).toBeUndefined();
  });

  it("pin a Chromium id that matches the one the host registers for", () => {
    const der = Buffer.from(identity.chromiumKey, "base64");
    const id = createHash("sha256")
      .update(der)
      .digest("hex")
      .slice(0, 32)
      .replace(/./g, (c: string) => String.fromCharCode("a".charCodeAt(0) + parseInt(c, 16)));
    expect(id).toBe(identity.chromiumId);
    expect((manifestFor("chromium") as any).key).toBe(identity.chromiumKey);
    // The Rust side has the same constant.
    const rust = readFileSync(new URL("../../crates/host/src/register.rs", import.meta.url), "utf8");
    expect(rust).toContain(`"${identity.chromiumId}"`);
  });
});
