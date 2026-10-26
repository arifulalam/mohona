import { describe, expect, it } from "vitest";
import { DEFAULTS, sanitize } from "../src/settings";
import { reconcile } from "../src/settingsync";

const at = (updated: number, more = {}) => ({ ...DEFAULTS, updated, ...more });

describe("settings sync", () => {
  it("does nothing when neither side was ever edited", () => {
    expect(reconcile(DEFAULTS, null)).toEqual({});
    expect(reconcile(DEFAULTS, "not json")).toEqual({});
  });

  it("gives Mohona the local copy when it has none, or an unusable one", () => {
    const local = at(10, { intercept: false });
    expect(reconcile(local, null)).toEqual({ push: local });
    expect(reconcile(local, "{")).toEqual({ push: local });
    expect(reconcile(local, "7")).toEqual({ push: local });
  });

  it("the newer copy wins", () => {
    const local = at(10);
    const newer = at(20, { excludedHosts: ["a.org"] });
    expect(reconcile(local, JSON.stringify(newer))).toEqual({ apply: newer });
    expect(reconcile(at(30), JSON.stringify(newer))).toEqual({ push: at(30) });
    expect(reconcile(newer, JSON.stringify(newer))).toEqual({});
  });

  it("cleans what came from Mohona", () => {
    const plan = reconcile(at(1), JSON.stringify({ updated: 5, intercept: "yes", minSizeBytes: -1 }));
    expect(plan.apply).toEqual(sanitize({ updated: 5 }));
  });
});
