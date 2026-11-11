// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { duplicateNote } from "../src/widgets";

const say = (text: string, vars?: Record<string, string>) =>
  vars ? text.replace(/\{(\w+)\}/g, (w, n: string) => vars[n] ?? w) : text;
const act = () => ({ open: vi.fn(async (): Promise<string | null> => null), reveal: vi.fn(async (): Promise<string | null> => null), again: vi.fn(async () => {}) });
const buttons = (el: HTMLElement) => [...el.querySelectorAll("button")].map((b) => b.textContent);

describe("a link downloaded before", () => {
  it("offers Open and Show in folder when the file is there", () => {
    const note = duplicateNote(say, { kind: "duplicate", message: "m", id: 3, status: "completed", exists: true }, act());
    expect(buttons(note)).toEqual(["Open", "Show in folder"]);
  });

  it("warns and offers Download again when the file is gone", async () => {
    const a = act();
    const note = duplicateNote(say, { kind: "duplicate", message: "m", id: 3, status: "completed", exists: false }, a);
    expect(note.textContent).toContain("no longer in its folder");
    expect(buttons(note)).toEqual(["Download again"]);
    note.querySelector("button")!.click();
    await Promise.resolve();
    expect(a.again).toHaveBeenCalled();
  });

  it("offers Download again after a failure, and only words while it runs", () => {
    expect(buttons(duplicateNote(say, { kind: "duplicate", message: "m", id: 3, status: "failed", exists: false }, act()))).toEqual(["Download again"]);
    expect(buttons(duplicateNote(say, { kind: "duplicate", message: "m", id: 3, status: "downloading", exists: false }, act()))).toEqual([]);
  });

  it("says what went wrong when opening fails", async () => {
    const a = act();
    a.open.mockResolvedValue("no program");
    const note = duplicateNote(say, { kind: "duplicate", message: "m", id: 3, status: "completed", exists: true }, a);
    note.querySelector("button")!.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(note.textContent).toContain("Could not open it: no program");
  });
});
