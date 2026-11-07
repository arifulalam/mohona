import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DropTarget } from "../src/DropTarget";
import { linksIn } from "../src/droptarget";
import { linkIn } from "../src/clipboard";
import type { Api } from "../src/types";

function api() {
  return { add: vi.fn(async () => 1), showMain: vi.fn(async () => undefined) } as unknown as Api & {
    add: ReturnType<typeof vi.fn>;
    showMain: ReturnType<typeof vi.fn>;
  };
}

const drop = (types: Record<string, string>) => ({
  dataTransfer: { getData: (type: string) => types[type] ?? "" },
});

afterEach(cleanup);

describe("DropTarget", () => {
  it("adds a link dropped on it, and says so", async () => {
    const a = api();
    render(<DropTarget api={a} />);
    fireEvent.drop(screen.getByRole("button"), drop({ "text/uri-list": "# from the browser\nhttps://x.test/a.zip" }));
    await waitFor(() => expect(a.add).toHaveBeenCalledWith("https://x.test/a.zip", null));
    expect(await screen.findByText("Added")).toBeTruthy();
  });

  it("takes several links, magnet links and plain text, each once", async () => {
    const a = api();
    render(<DropTarget api={a} />);
    fireEvent.drop(
      screen.getByRole("button"),
      drop({ "text/plain": "https://x.test/1.zip\nhttps://x.test/1.zip\nmagnet:?xt=urn:btih:abc\nhello" }),
    );
    await waitFor(() => expect(a.add).toHaveBeenCalledTimes(2));
    expect(a.add).toHaveBeenNthCalledWith(2, "magnet:?xt=urn:btih:abc", null);
    expect(await screen.findByText("Added 2")).toBeTruthy();
  });

  it("refuses what is not a link and adds nothing", async () => {
    const a = api();
    render(<DropTarget api={a} />);
    fireEvent.drop(screen.getByRole("button"), drop({ "text/plain": "just some words" }));
    expect(await screen.findByText("Not a link")).toBeTruthy();
    expect(a.add).not.toHaveBeenCalled();
  });

  it("says when the engine would not take it, and opens Mohona on a click", async () => {
    const a = api();
    a.add.mockRejectedValueOnce(new Error("no engine"));
    render(<DropTarget api={a} />);
    fireEvent.drop(screen.getByRole("button"), drop({ "text/plain": "https://x.test/a.zip" }));
    expect(await screen.findByText("Not added")).toBeTruthy();
    fireEvent.click(screen.getByRole("button"));
    expect(a.showMain).toHaveBeenCalled();
  });

  it("reads one link per line", () => {
    expect(linksIn("a\n\n# c\nhttps://y.test/f", linkIn)).toEqual(["https://y.test/f"]);
    expect(linksIn(null, linkIn)).toEqual([]);
  });
});
