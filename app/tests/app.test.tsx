import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../src/App";
import type { Api, Config, ConfigChange, EngineStatus, ExtensionSettings, Plan, PlanWindow, Row } from "../src/types";

const row = (id: number, name: string, status: string, extra: Partial<Row> = {}): Row => ({
  id, name, url: `https://x.test/${name}`, status, downloaded: 50, size: 100, percent: 50, speed: 0, eta: null,
  error: null, path: null, expired: false, ...extra,
});

function fake(initial: Row[], running = true) {
  let rows = initial;
  let engine = running;
  let plan: Plan = { windows: [], openNow: true, action: null, countdown: null, message: null };
  let cfg: Config = { downloadFolder: "C:\\Dl", defaultFolder: "C:\\Dl", speedLimit: null, maxActive: 4, notifications: true, keepInTray: true };
  let ext: ExtensionSettings = { intercept: true, minSizeBytes: 0, excludedHosts: [], videoButton: true, buttonHiddenHosts: [], autoRefresh: true };
  const status = (): EngineStatus => ({ running: engine, version: engine ? "9.9" : null, message: null });
  const api = {
    status: vi.fn(async () => status()),
    start: vi.fn(async () => ((engine = true), status())),
    list: vi.fn(async () => rows),
    add: vi.fn(async (url: string) => {
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
    plan: vi.fn(async () => plan),
    setPlan: vi.fn(async (windows: PlanWindow[]) => (plan = { ...plan, windows })),
    setAfterAction: vi.fn(async (action: string | null) => (plan = { ...plan, action, countdown: null })),
    cancelAction: vi.fn(async () => (plan = { ...plan, action: null, countdown: null })),
    config: vi.fn(async () => cfg),
    setConfig: vi.fn(async (c: ConfigChange) => {
      if (c.maxActive === 13) throw new Error("the engine refused this");
      return (cfg = { ...cfg, speedLimit: c.speedLimit || null, maxActive: c.maxActive ?? cfg.maxActive });
    }),
    extensionSettings: vi.fn(async () => ext),
    setExtensionSettings: vi.fn(async (s: ExtensionSettings) => (ext = s)),
  } satisfies Api;
  return api;
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
    await userEvent.type(within(dialog).getByRole("textbox"), "ftp-bad-address");
    await userEvent.click(within(dialog).getByRole("button", { name: "Download" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain("must start with http");
    await userEvent.clear(within(dialog).getByRole("textbox"));
    await userEvent.type(within(dialog).getByRole("textbox"), "https://x.test/new.zip");
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
    expect(api.setConfig).toHaveBeenCalledWith({ downloadFolder: "", speedLimit: 2 * 1024 * 1024, maxActive: 6, notifications: true, keepInTray: false });
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
});
