import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../src/App";
import type { Api, Category, Config, ConfigChange, EngineStatus, ExtensionSettings, Details, Plan, PlanWindow, Queue, Row, Rule } from "../src/types";

const row = (id: number, name: string, status: string, extra: Partial<Row> = {}): Row => ({
  id, name, url: `https://x.test/${name}`, status, downloaded: 50, size: 100, percent: 50, speed: 0, eta: null,
  error: null, path: null, expired: false, ...extra,
});

function fake(initial: Row[], running = true) {
  let rows = initial;
  let clip: string | null = null;
  let engine = running;
  let cats: Category[] = [{ id: 1, name: "Compressed", extensions: ["zip"], folder: null }];
  let plan: Plan = { windows: [], openNow: true, action: null, countdown: null, message: null };
  let cfg: Config = { downloadFolder: "C:\\Dl", defaultFolder: "C:\\Dl", speedLimit: null, maxActive: 4, notifications: true, keepInTray: true, sortByType: false };
  let ext: ExtensionSettings = { intercept: true, minSizeBytes: 0, excludedHosts: [], videoButton: true, buttonHiddenHosts: [], autoRefresh: true };
  let queues: Queue[] = [{ id: 1, name: "Main", maxActive: 4, speedLimit: null, windows: [], openNow: true }];
  let rules: Rule[] = [];
  const status = (): EngineStatus => ({ running: engine, version: engine ? "9.9" : null, message: null });
  const api = {
    status: vi.fn(async () => status()),
    start: vi.fn(async () => ((engine = true), status())),
    list: vi.fn(async () => rows),
    add: vi.fn(async (url: string, _folder?: string | null, _queue?: number) => {
      if (url.includes("bad")) throw new Error("the address must start with http://");
      rows = [...rows, row(99, "new.zip", "downloading")];
      return 99;
    }),
    pause: vi.fn(async (id: number) => void (rows = rows.map((r) => (r.id === id ? { ...r, status: "paused" } : r)))),
    resume: vi.fn(async () => undefined),
    cancel: vi.fn(async () => undefined),
    remove: vi.fn(async (id: number) => void (rows = rows.filter((r) => r.id !== id))),
    open: vi.fn(async (id: number) => {
      if (id === 7) throw new Error("for safety, programs and scripts are not opened from here");
    }),
    show: vi.fn(async () => undefined),
    details: vi.fn(async (id: number): Promise<Details> => {
      const r = rows.find((x) => x.id === id);
      if (!r) throw new Error("there is no such download");
      return {
        id, status: r.status, downloaded: 50, size: 100, speed: 2048,
        connections: [
          { number: 1, start: 0, end: 50, received: 50, speed: 0, state: "done" },
          { number: 2, start: 50, end: 100, received: 10, speed: 2048, state: "active" },
        ],
      };
    }),
    rules: vi.fn(async () => rules),
    setRules: vi.fn(async (next: Rule[]) => {
      if (next.some((r) => r.folder?.includes(".."))) throw new Error("rule: a folder name inside the download folder cannot use \"..\"");
      let id = rules.reduce((m, r) => Math.max(m, r.id), 0);
      return (rules = next.map((r) => ({ ...r, id: r.id || ++id })));
    }),
    queues: vi.fn(async () => queues),
    addQueue: vi.fn(async (name: string) => {
      if (name === "Main") throw new Error("there is already a queue with that name");
      return (queues = [...queues, { id: 2, name, maxActive: 3, speedLimit: null, windows: [], openNow: true }]);
    }),
    renameQueue: vi.fn(async (id: number, name: string) => (queues = queues.map((q) => (q.id === id ? { ...q, name } : q)))),
    removeQueue: vi.fn(async (id: number) => (queues = queues.filter((q) => q.id !== id))),
    setQueue: vi.fn(async (id: number, c: { maxActive?: number; speedLimit?: number }) =>
      (queues = queues.map((q) => (q.id === id ? { ...q, maxActive: c.maxActive ?? q.maxActive, speedLimit: c.speedLimit || null } : q)))),
    plan: vi.fn(async () => plan),
    setPlan: vi.fn(async (windows: PlanWindow[]) => (plan = { ...plan, windows })),
    setAfterAction: vi.fn(async (action: string | null) => (plan = { ...plan, action, countdown: null })),
    cancelAction: vi.fn(async () => (plan = { ...plan, action: null, countdown: null })),
    clipboard: vi.fn(async (): Promise<string | null> => clip),
    pickFolder: vi.fn(async () => "D:\\Picked"),
    categories: vi.fn(async () => cats),
    addCategory: vi.fn(async (name: string, extensions: string[], folder: string | null) => {
      if (extensions.includes("zip")) throw new Error("\"zip\" already belongs to Compressed");
      cats = [...cats, { id: 2, name, extensions, folder }];
      return cats;
    }),
    removeCategory: vi.fn(async (id: number) => (cats = cats.filter((c) => c.id !== id))),
    config: vi.fn(async () => cfg),
    setConfig: vi.fn(async (c: ConfigChange) => {
      if (c.maxActive === 13) throw new Error("the engine refused this");
      return (cfg = { ...cfg, speedLimit: c.speedLimit || null, maxActive: c.maxActive ?? cfg.maxActive });
    }),
    extensionSettings: vi.fn(async () => ext),
    setExtensionSettings: vi.fn(async (s: ExtensionSettings) => (ext = s)),
  } satisfies Api;
  return Object.assign(api, { copy: (text: string | null) => void (clip = text) });
}

afterEach(cleanup);

describe("App", () => {
  it("lists downloads with their state and the engine's state", async () => {
    render(<App api={fake([row(1, "a.iso", "downloading", { speed: 2048, eta: 30 }), row(2, "b.zip", "completed")])} every={50} />);
    expect(await screen.findByText("a.iso")).toBeTruthy();
    expect(screen.getByText("2.0 KB/s")).toBeTruthy();
    expect(screen.getByText("30s left")).toBeTruthy();
    expect(within(screen.getByRole("listbox")).getByText("Finished")).toBeTruthy();
    expect(screen.getByText(/Engine running \(9.9\)/)).toBeTruthy();
  });

  it("offers to start the engine when it is not running, and disables adding until it is", async () => {
    const api = fake([], false);
    render(<App api={api} every={50} />);
    const start = await screen.findByRole("button", { name: "Start Mohona" });
    expect((screen.getByRole("button", { name: "+ Add" }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(start);
    await waitFor(() => expect(api.start).toHaveBeenCalled());
    await waitFor(() => expect((screen.getByRole("button", { name: "+ Add" }) as HTMLButtonElement).disabled).toBe(false));
  });

  it("filters by the sidebar", async () => {
    render(<App api={fake([row(1, "a.iso", "downloading"), row(2, "b.zip", "completed")])} every={50} />);
    await screen.findByText("a.iso");
    await userEvent.click(screen.getByRole("button", { name: /Finished/ }));
    expect(screen.queryByText("a.iso")).toBeNull();
    expect(screen.getByText("b.zip")).toBeTruthy();
  });

  it("pauses the selected download and only enables fitting buttons", async () => {
    const api = fake([row(1, "a.iso", "downloading")]);
    render(<App api={api} every={50} />);
    const pause = screen.getByRole("button", { name: "Pause" }) as HTMLButtonElement;
    expect(pause.disabled).toBe(true);
    await userEvent.click(await screen.findByText("a.iso"));
    expect(pause.disabled).toBe(false);
    expect((screen.getByRole("button", { name: "Resume" }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(pause);
    await waitFor(() => expect(api.pause).toHaveBeenCalledWith(1));
    await waitFor(() => expect(screen.getByText("Paused")).toBeTruthy());
  });

  it("adds addresses and shows the engine's reason when one is refused", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "+ Add" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Addresses (one per line)" }), "ftp-bad-address");
    await userEvent.click(within(dialog).getByRole("button", { name: "Download" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("must start with http");
    await userEvent.clear(within(dialog).getByRole("textbox", { name: "Addresses (one per line)" }));
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Addresses (one per line)" }), "https://x.test/new.zip");
    await userEvent.click(within(dialog).getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.add).toHaveBeenCalledWith("https://x.test/new.zip", null));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText("new.zip")).toBeTruthy();
  });

  it("asks before removing, and can delete the file too", async () => {
    const api = fake([row(1, "a.iso", "completed")]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByText("a.iso"));
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = screen.getByRole("dialog");
    expect(api.remove).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(api.remove).toHaveBeenCalledWith(1, true));
    await waitFor(() => expect(screen.queryByText("a.iso")).toBeNull());
  });

  it("shows an expired link in words a person understands", async () => {
    render(<App api={fake([row(1, "a.iso", "failed", { expired: true, error: "server answered 403" })])} every={50} />);
    expect(await screen.findByText(/link has expired/)).toBeTruthy();
  });

  it("saves the settings: limit, simultaneous downloads, and the browser's list of sites", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(await within(dialog).findByLabelText("Speed limit"), "2");
    const active = within(dialog).getByLabelText("Simultaneous downloads");
    await userEvent.clear(active);
    await userEvent.type(active, "6");
    await userEvent.type(within(dialog).getByLabelText("Smallest size in MB"), "5");
    await userEvent.type(within(dialog).getByPlaceholderText("example.com"), "bank.example");
    await userEvent.click(within(dialog).getByRole("checkbox", { name: /video/ }));
    await userEvent.click(within(dialog).getByRole("checkbox", { name: /tray/ }));
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.setConfig).toHaveBeenCalledWith({ downloadFolder: "", speedLimit: 2 * 1024 * 1024, maxActive: 6, notifications: true, keepInTray: false, sortByType: false });
    expect(api.setExtensionSettings).toHaveBeenCalledWith(
      expect.objectContaining({ minSizeBytes: 5 * 1024 * 1024, excludedHosts: ["bank.example"], videoButton: false }),
    );
  });

  it("keeps the settings open and shows the reason when something is refused", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(await within(dialog).findByLabelText("Speed limit"), "abc");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("must be a number");
    expect(api.setConfig).not.toHaveBeenCalled();
    await userEvent.clear(within(dialog).getByLabelText("Speed limit"));
    const active = within(dialog).getByLabelText("Simultaneous downloads");
    await userEvent.clear(active);
    await userEvent.type(active, "13");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("engine refused");
    expect(api.setExtensionSettings).not.toHaveBeenCalled();
  });

  it("opens finished files, shows them in their folder, and shows the reason when opening is refused", async () => {
    const api = fake([row(1, "a.pdf", "completed"), row(7, "setup.exe", "completed"), row(2, "b.iso", "downloading")]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Open a.pdf" }));
    await waitFor(() => expect(api.open).toHaveBeenCalledWith(1));
    await userEvent.click(screen.getByRole("button", { name: "Show a.pdf in its folder" }));
    await waitFor(() => expect(api.show).toHaveBeenCalledWith(1));
    await userEvent.click(screen.getByRole("button", { name: "Open setup.exe" }));
    expect((await screen.findByRole("alert")).textContent).toContain("programs and scripts");
    expect(screen.queryByRole("button", { name: "Open b.iso" })).toBeNull();
  });

  it("saves a schedule window and the end action", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Schedule" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(await within(dialog).findByRole("button", { name: "Add window" }));
    expect(within(dialog).getByText("Every day 23:00–06:00")).toBeTruthy();
    await userEvent.selectOptions(within(dialog).getByLabelText("When everything is done"), "shutdown");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.setPlan).toHaveBeenCalledWith([{ days: [], start: "23:00", end: "06:00", speedLimit: null }]);
    expect(api.setAfterAction).toHaveBeenCalledWith("shutdown", 60);
  });

  it("refuses a bad window with a reason and keeps the dialog open", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Schedule" }));
    const dialog = await screen.findByRole("dialog");
    const end = await within(dialog).findByLabelText("Window end");
    await userEvent.clear(end);
    await userEvent.type(end, "23:00");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add window" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("same time");
    expect(api.setPlan).not.toHaveBeenCalled();
  });

  it("shows the countdown and lets the person cancel it", async () => {
    const api = fake([]);
    await api.setAfterAction("shutdown");
    await api.plan();
    api.plan.mockImplementation(async () => ({ windows: [], openNow: true, action: "shutdown", countdown: 42, message: null }));
    render(<App api={api} every={50} />);
    expect(await screen.findByText(/Shut the computer down in 42 s/)).toBeTruthy();
    await userEvent.click(screen.getAllByRole("button", { name: "Cancel" }).find((b) => b.closest("[role=status]"))!);
    await waitFor(() => expect(api.cancelAction).toHaveBeenCalled());
  });

  it("adds to a chosen folder with the folder dialog", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "+ Add" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect((within(dialog).getByLabelText("Save in folder") as HTMLInputElement).value).toBe("D:\\Picked"));
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Addresses (one per line)" }), "https://x.test/a.zip");
    await userEvent.click(within(dialog).getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.add).toHaveBeenCalledWith("https://x.test/a.zip", "D:\\Picked"));
  });

  it("lists, adds and removes categories, and shows why one is refused", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Categories" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("Compressed")).toBeTruthy();
    await userEvent.type(within(dialog).getByLabelText("Category name"), "Archives");
    await userEvent.type(within(dialog).getByLabelText("File types"), ".ZIP, tar");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add category" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("already belongs to Compressed");
    const types = within(dialog).getByLabelText("File types");
    await userEvent.clear(types);
    await userEvent.type(types, "7z tar");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add category" }));
    await waitFor(() => expect(api.addCategory).toHaveBeenLastCalledWith("Archives", ["7z", "tar"], null));
    expect(await within(dialog).findByText("Archives")).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove category Archives" }));
    await waitFor(() => expect(within(dialog).queryByText("Archives")).toBeNull());
  });

  it("lists rules, adds one with checks, switches it off, reorders and removes it", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Rules" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("No rules yet.")).toBeTruthy();
    // Checks come before anything is sent.
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("name");
    await userEvent.type(within(dialog).getByLabelText("Rule name"), "Zips");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("at least one condition");
    await userEvent.type(within(dialog).getByLabelText("File types (such as zip, iso)"), ".ZIP, 7z");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("Choose what to do");
    expect(api.setRules).not.toHaveBeenCalled();
    await userEvent.type(within(dialog).getByLabelText("Save in this folder (optional)"), "Archives");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    await waitFor(() => expect(api.setRules).toHaveBeenLastCalledWith([expect.objectContaining({ name: "Zips", extensions: ["zip", "7z"], folder: "Archives", block: false })]));
    expect(await within(dialog).findByText("Types: zip, 7z → Folder: Archives")).toBeTruthy();
    // A second one that blocks, then move it up.
    await userEvent.type(within(dialog).getByLabelText("Rule name"), "No ads");
    await userEvent.type(within(dialog).getByLabelText("Site (such as example.com)"), "ads.example.com");
    await userEvent.click(within(dialog).getByLabelText("Block this download"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    await waitFor(() => expect(within(dialog).getAllByRole("listitem")).toHaveLength(2));
    await userEvent.click(within(dialog).getAllByRole("button", { name: "Move up" })[1]!);
    await waitFor(() => expect(api.setRules.mock.lastCall![0].map((r: Rule) => r.name)).toEqual(["No ads", "Zips"]));
    await userEvent.click(await within(dialog).findByLabelText("Use rule “No ads”"));
    await waitFor(() => expect(api.setRules.mock.lastCall![0][0]?.enabled).toBe(false));
    await userEvent.click(within(dialog).getAllByRole("button", { name: "Remove" })[0]!);
    await waitFor(() => expect(within(dialog).getAllByRole("listitem")).toHaveLength(1));
    // The engine's own refusal is shown.
    await userEvent.type(within(dialog).getByLabelText("Rule name"), "Up");
    await userEvent.type(within(dialog).getByLabelText("Address contains"), "x");
    await userEvent.type(within(dialog).getByLabelText("Save in this folder (optional)"), "../up");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("cannot use");
  });

  it("adds a queue, changes its limits, and sends new downloads to it", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Queues" }));
    let dialog = await screen.findByRole("dialog");
    await userEvent.type(await within(dialog).findByLabelText("New queue name"), "Main");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("already a queue");
    await userEvent.clear(within(dialog).getByLabelText("New queue name"));
    await userEvent.type(within(dialog).getByLabelText("New queue name"), "Night");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add" }));
    await waitFor(() => expect(api.addQueue).toHaveBeenLastCalledWith("Night"));
    const active = await within(dialog).findByLabelText("Queue simultaneous downloads");
    await userEvent.clear(active);
    await userEvent.type(active, "2");
    await userEvent.type(within(dialog).getByLabelText("Queue speed limit"), "1");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.setQueue).toHaveBeenCalledWith(2, expect.objectContaining({ maxActive: 2, speedLimit: 1024 * 1024 })));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await userEvent.click(screen.getByRole("button", { name: "+ Add" }));
    dialog = await screen.findByRole("dialog");
    await userEvent.selectOptions(await within(dialog).findByLabelText("Queue"), "Night");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Addresses (one per line)" }), "https://x.test/a.zip");
    await userEvent.click(within(dialog).getByRole("button", { name: "Download" }));
    await waitFor(() => expect(api.add).toHaveBeenCalledWith("https://x.test/a.zip", null, 2));
  });

  it("shows each connection of a running download and says so when there are none", async () => {
    const api = fake([row(1, "big.iso", "downloading"), row(2, "done.zip", "completed")]);
    render(<App api={api} every={50} />);
    await userEvent.click(await screen.findByRole("button", { name: "Details of big.iso" }));
    const dialog = await screen.findByRole("dialog", { name: "Download details" });
    expect(await within(dialog).findByText("Connections (1 receiving of 2)")).toBeTruthy();
    expect(within(dialog).getByLabelText("Connection 1 progress")).toBeTruthy();
    expect(within(dialog).getByText("Receiving")).toBeTruthy();
    expect(within(dialog).getByText("Done")).toBeTruthy();
    expect(within(dialog).getAllByText("2.0 KB/s").length).toBeGreaterThan(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // Finished downloads have no details button.
    expect(screen.queryByRole("button", { name: "Details of done.zip" })).toBeNull();
  });

  it("lists downloads by category and sorts them", async () => {
    const api = fake([row(1, "a.zip", "completed"), row(2, "movie.mp4", "completed"), row(3, "b.zip", "completed"), row(4, "notes.txt", "completed")]);
    render(<App api={api} every={50} />);
    const list = async () => within(await screen.findByRole("listbox", { name: "Downloads" })).getAllByRole("option").map((o) => o.textContent ?? "");
    await waitFor(async () => expect((await list()).length).toBe(4));
    await userEvent.click(await screen.findByRole("button", { name: "Category Compressed, 2" }));
    await waitFor(async () => expect((await list()).length).toBe(2));
    expect((await list()).every((t) => t.includes("zip"))).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Category Compressed, 2" }));
    await userEvent.click(screen.getByRole("button", { name: /^All types/ }));
    await userEvent.selectOptions(screen.getByLabelText("Sort by"), "Name");
    await waitFor(async () => expect((await list())[0]).toContain("a.zip"));
    expect((await list())[3]).toContain("notes.txt");
    await userEvent.selectOptions(screen.getByLabelText("Sort by"), "Category and type");
    await waitFor(async () => expect((await list()).map((t) => t.slice(0, 7))).toEqual(["a.zipCo", "b.zipCo", "movie.m", "notes.t"]));
  });

  it("changes the colour theme from the list of themes and remembers it", async () => {
    const api = fake([]);
    render(<App api={api} every={50} />);
    await userEvent.selectOptions(await screen.findByLabelText("Colour theme"), "Forest");
    expect(document.documentElement.getAttribute("data-theme")).toBe("forest");
    expect(localStorage.getItem("mohona.theme")).toBe("forest");
    await userEvent.selectOptions(screen.getByLabelText("Colour theme"), "System");
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("offers a copied link once, only when asked to, and adds it on request", async () => {
    localStorage.setItem("mohona.clipboard", "on");
    const api = fake([row(1, "a.zip", "completed", { url: "https://x.test/seen.zip" })]);
    api.copy("https://x.test/new.zip");
    render(<App api={api} every={50} />);
    expect(await screen.findByText(/You copied a link: https:\/\/x\.test\/new\.zip/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Ignore" }));
    expect(screen.queryByText(/You copied a link/)).toBeNull();
    window.dispatchEvent(new Event("focus"));
    await new Promise((r) => setTimeout(r, 150));
    expect(screen.queryByText(/You copied a link/)).toBeNull();
    api.copy("https://x.test/second.zip");
    window.dispatchEvent(new Event("focus"));
    await userEvent.click(await screen.findByRole("button", { name: "Download it" }));
    await waitFor(() => expect(api.add).toHaveBeenCalledWith("https://x.test/second.zip", null));
    api.copy("https://x.test/seen.zip");
    window.dispatchEvent(new Event("focus"));
    await new Promise((r) => setTimeout(r, 150));
    expect(screen.queryByText(/You copied a link/)).toBeNull();
    localStorage.removeItem("mohona.clipboard");
  });

  it("does not look at the clipboard unless the switch is on", async () => {
    localStorage.removeItem("mohona.clipboard");
    const api = fake([]);
    api.copy("https://x.test/new.zip");
    render(<App api={api} every={50} />);
    await screen.findByRole("button", { name: "+ Add" });
    window.dispatchEvent(new Event("focus"));
    await new Promise((r) => setTimeout(r, 150));
    expect(api.clipboard).not.toHaveBeenCalled();
  });
});
