import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { signal } from "@preact/signals";
import type { DataSource, Connection, Refusal, ViewName } from "../data";
import type { ViewsGetResult, ViewRow } from "../gen/protocol";
import { App } from "../App";

const row = (uid: string, title: string, extra: Partial<ViewRow> = {}): ViewRow => ({
  uid,
  title,
  completed: false,
  conflict: false,
  depth: 0,
  due_today: false,
  overdue: false,
  pending: false,
  repeats: false,
  kind: "Task",
  list: { id: "c1", name: "Inbox" },
  ...extra,
});

const mkView = (title: string, rows: ViewRow[], due: string | null = null): ViewsGetResult => ({
  epoch: "e",
  revision: 1,
  now: "2026-11-17T09:00:00+01:00",
  today: "2026-11-17",
  title,
  open_count: rows.length,
  sections: [{ group: "today", label: null, rows }],
  capture: { collection: "c1", name: "Inbox", due },
});

let refusal: (r: Refusal) => void = () => {};

function fake(over: Partial<Record<string, unknown>> = {}) {
  const inbox = signal<ViewsGetResult | undefined>(
    mkView("Inbox", [row("u1", "Buy milk", { note: "2%" }), row("u2", "Call the plumber")]),
  );
  const today = signal<ViewsGetResult | undefined>(mkView("Today", [row("u3", "Pay the invoice")], "2026-11-17"));
  const connection = signal<Connection>("open");
  const fresh = signal(true);
  const ds = {
    view: (n: ViewName) => (n === "today" ? today : inbox),
    connection,
    fresh,
    refresh: vi.fn(async () => {}),
    complete: vi.fn(async () => ({ outcome: "applied", op_id: "o", revision: 2, message: "ok" })),
    create: vi.fn(async () => ({ outcome: "applied", op_id: "o", revision: 2, message: "ok" })),
    undo: vi.fn(async () => ({ outcome: "applied", op_id: "o", revision: 3, message: "ok" })),
    onRefusal: vi.fn((cb: (r: Refusal) => void) => {
      refusal = cb;
      return () => {};
    }),
    dispose: vi.fn(),
    ...over,
  } as unknown as DataSource & { complete: any; create: any; undo: any };
  return { ds, connection, fresh, inbox };
}

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("tabs", () => {
  it("tabs are Inbox then Today with aria-labels", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("aria-label"))).toEqual(["Inbox", "Today"]);
    expect(tabs[0]!.getAttribute("aria-selected")).toBe("true");
  });

  it("calendar icon shows today's day", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    expect(screen.getByLabelText("Today").textContent).toContain("17");
  });

  it("remembers last tab", () => {
    const { ds } = fake();
    const first = render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Today"));
    expect(localStorage.getItem("twydo.tab")).toBe("today");
    first.unmount();
    render(<App ds={ds} />);
    expect(screen.getByLabelText("Today").getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Today");
  });
});

describe("completing", () => {
  it("completed row is checked and disabled", () => {
    const { ds } = fake({
      view: () =>
        signal(mkView("Inbox", [row("u9", "Already done", { completed: true }), row("u8", "Still open")])),
    });
    render(<App ds={ds} />);
    const box = screen.getByLabelText("Complete Already done") as HTMLButtonElement;
    expect(box.getAttribute("aria-checked")).toBe("true");
    expect(box.disabled).toBe(true);
    const open = screen.getByLabelText("Complete Still open") as HTMLButtonElement;
    expect(open.getAttribute("aria-checked")).toBe("false");
    expect(open.disabled).toBe(false);
  });

  it("tapping checkbox completes and shows undo banner", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Complete Buy milk"));
    expect(ds.complete).toHaveBeenCalledWith("u1");
    expect(await screen.findByText('Completed "Buy milk"')).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(ds.undo).toHaveBeenCalled();
  });

  it("a repeating row that stays in the view is open again after the newer view", async () => {
    const { ds, inbox } = fake();
    render(<App ds={ds} />);
    const box = screen.getByLabelText("Complete Buy milk") as HTMLButtonElement;
    fireEvent.click(box);
    await screen.findByText('Completed "Buy milk"');
    expect(box.getAttribute("aria-checked")).toBe("true");
    expect(box.disabled).toBe(true);
    // The next occurrence: same uid, open again, in a view at the write's revision.
    inbox.value = {
      ...mkView("Inbox", [row("u1", "Buy milk", { repeats: true }), row("u2", "Call the plumber")]),
      revision: 2,
    };
    await waitFor(() => expect(box.getAttribute("aria-checked")).toBe("false"));
    expect(box.disabled).toBe(false);
  });

  it("a queued write later refused unticks the row and shows the message", async () => {
    const { ds } = fake({
      complete: vi.fn(async () => ({ outcome: "queued", op_id: "o", revision: 2, message: "queued" })),
    });
    render(<App ds={ds} />);
    const box = screen.getByLabelText("Complete Buy milk") as HTMLButtonElement;
    fireEvent.click(box);
    await screen.findByText('Completed "Buy milk"');
    expect(box.getAttribute("aria-checked")).toBe("true");
    refusal({ kind: "complete", uid: "u1", message: "The task was deleted on the server" });
    expect(await screen.findByText("The task was deleted on the server")).toBeTruthy();
    await waitFor(() => expect(box.getAttribute("aria-checked")).toBe("false"));
  });

  it("a refused queued capture offers its text back", async () => {
    const { ds } = fake({
      create: vi.fn(async () => ({ outcome: "queued", op_id: "o", revision: 2, message: "queued" })),
    });
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("New task"));
    fireEvent.input(screen.getByLabelText("Title"), { target: { value: "Water plants" } });
    fireEvent.input(screen.getByLabelText("Note"), { target: { value: "ferns too" } });
    fireEvent.click(screen.getByText("Add"));
    await screen.findByText('Added "Water plants" to Inbox');
    refusal({
      kind: "create",
      title: "Water plants",
      note: "ferns too",
      capture: { collection: "c1", name: "Inbox", due: null },
      message: "The list was deleted",
    });
    expect(await screen.findByText('Could not add "Water plants": The list was deleted')).toBeTruthy();
    fireEvent.click(screen.getByText("Edit"));
    expect((await screen.findByLabelText("Title") as HTMLInputElement).value).toBe("Water plants");
    expect((screen.getByLabelText("Note") as HTMLTextAreaElement).value).toBe("ferns too");
  });

  it("a refused queued undo shows a banner", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    refusal({ kind: "undo", message: "Nothing to undo" });
    expect(await screen.findByText("Could not undo: Nothing to undo")).toBeTruthy();
  });

  it("new banner replaces old", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Complete Buy milk"));
    await screen.findByText('Completed "Buy milk"');
    fireEvent.click(screen.getByLabelText("Complete Call the plumber"));
    await screen.findByText('Completed "Call the plumber"');
    expect(screen.queryByText('Completed "Buy milk"')).toBeNull();
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });

  it("refusal unticks and shows message", async () => {
    const { ds } = fake({
      complete: vi.fn(async () => {
        throw new Error("The task changed on the server");
      }),
    });
    render(<App ds={ds} />);
    const box = screen.getByLabelText("Complete Buy milk") as HTMLButtonElement;
    fireEvent.click(box);
    expect(await screen.findByText("The task changed on the server")).toBeTruthy();
    await waitFor(() => expect(box.getAttribute("aria-checked")).toBe("false"));
  });

  it("banner disappears after 4 seconds", async () => {
    vi.useFakeTimers();
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Complete Buy milk"));
    await vi.advanceTimersByTimeAsync(10);
    expect(screen.queryByText('Completed "Buy milk"')).not.toBeNull();
    await vi.advanceTimersByTimeAsync(4100);
    expect(screen.queryByText('Completed "Buy milk"')).toBeNull();
  });
});

describe("capture", () => {
  it("capture from Today sends due today", async () => {
    const { ds } = fake();
    localStorage.setItem("twydo.tab", "today");
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("New task"));
    fireEvent.input(screen.getByLabelText("Title"), { target: { value: "Water plants" } });
    fireEvent.input(screen.getByLabelText("Note"), { target: { value: "ferns" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(ds.create).toHaveBeenCalledWith("Water plants", "ferns", {
      collection: "c1",
      name: "Inbox",
      due: "2026-11-17",
    });
    expect(await screen.findByText('Added "Water plants" to Inbox')).toBeTruthy();
  });

  it("New task is disabled until the view and its capture target have arrived", () => {
    const inbox = signal<ViewsGetResult | undefined>(undefined);
    const { ds } = fake({ view: () => inbox });
    render(<App ds={ds} />);
    const fab = screen.getByLabelText("New task") as HTMLButtonElement;
    expect(fab.disabled).toBe(true);
    inbox.value = mkView("Inbox", [row("u1", "Buy milk")]);
    return waitFor(() => expect(fab.disabled).toBe(false));
  });

  it("blank title cannot be added", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("New task"));
    const add = screen.getByRole("button", { name: "Add" }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    fireEvent.input(screen.getByLabelText("Title"), { target: { value: "   " } });
    expect(add.disabled).toBe(true);
    fireEvent.input(screen.getByLabelText("Title"), { target: { value: "x" } });
    expect(add.disabled).toBe(false);
  });
});

describe("connection", () => {
  it("unpaired shows pairing", () => {
    const { ds, connection } = fake();
    connection.value = "unpaired";
    render(<App ds={ds} />);
    expect(screen.getByText("Add Twydo to your Home Screen first, then pair inside the app.")).toBeTruthy();
    expect(screen.getByLabelText("Pairing code")).toBeTruthy();
  });

  it("closed connection shows offline state", () => {
    const { ds, connection, fresh } = fake();
    connection.value = "closed";
    fresh.value = false;
    render(<App ds={ds} />);
    expect(screen.getByText("Can't reach Twydo")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(screen.getByText(/last updated/)).toBeTruthy();
    expect(screen.getByText("Buy milk")).toBeTruthy();
  });
});

describe("polish", () => {
  it("failed capture keeps the sheet and draft and shows the message", async () => {
    const { ds } = fake({
      create: vi.fn(async () => {
        throw new Error("No such list");
      }),
    });
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("New task"));
    fireEvent.input(screen.getByLabelText("Title"), { target: { value: "Keep me" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByText("No such list")).toBeTruthy();
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Keep me");
    expect((screen.getByRole("button", { name: "Add" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("shows a cached view while it refreshes, with no label and not dimmed", () => {
    const { ds, fresh } = fake();
    fresh.value = false;
    const { container } = render(<App ds={ds} />);
    expect(screen.getByText("Buy milk")).toBeTruthy();
    expect(screen.queryByText(/Updating/)).toBeNull();
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull();
    expect(container.querySelector("main")!.className).toBe("screen");
    expect(screen.queryByText("Can't reach Twydo")).toBeNull();
  });

  it("shows a spinner on the first load, then the view", async () => {
    const { ds, inbox } = fake();
    inbox.value = undefined;
    render(<App ds={ds} />);
    expect(screen.getByRole("status", { name: "Loading" })).toBeTruthy();
    inbox.value = mkView("Inbox", [row("u1", "Buy milk")]);
    expect(await screen.findByText("Buy milk")).toBeTruthy();
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull();
  });

  it("no spinner when offline before any view", () => {
    const { ds, inbox, connection } = fake();
    inbox.value = undefined;
    connection.value = "closed";
    render(<App ds={ds} />);
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull();
    expect(screen.getByText("Can't reach Twydo")).toBeTruthy();
  });

  it("a row a refresh dropped animates out, then goes", async () => {
    vi.useFakeTimers();
    const { ds, inbox } = fake();
    render(<App ds={ds} />);
    inbox.value = { ...mkView("Inbox", [row("u2", "Call the plumber"), row("u4", "New one")]), revision: 2 };
    await vi.advanceTimersByTimeAsync(0);
    const gone = screen.getByText("Buy milk").closest("li")!;
    expect(gone.classList.contains("leaving")).toBe(true);
    expect(gone.getAttribute("aria-hidden")).toBe("true");
    expect(screen.getByText("New one").closest("li")!.classList.contains("entering")).toBe(true);
    expect(screen.getByText("Call the plumber").closest("li")!.className).toBe("row");
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.queryByText("Buy milk")).toBeNull();
  });

  it("a completed row leaves ticked", async () => {
    vi.useFakeTimers();
    const { ds, inbox } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Complete Buy milk"));
    await vi.advanceTimersByTimeAsync(0);
    inbox.value = { ...mkView("Inbox", [row("u2", "Call the plumber")]), revision: 2 };
    await vi.advanceTimersByTimeAsync(0);
    const li = screen.getByText("Buy milk").closest("li")!;
    expect(li.className).toContain("leaving");
    expect(li.className).toContain("done");
  });

  it("switching tabs shows the other list without animating it in", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Today"));
    expect(screen.getByText("Pay the invoice").closest("li")!.className).toBe("row");
    expect(screen.queryByText("Buy milk")).toBeNull();
  });

  it("undo is disabled while a write is pending", async () => {
    let release!: () => void;
    const { ds } = fake();
    render(<App ds={ds} />);
    fireEvent.click(screen.getByLabelText("Complete Buy milk"));
    await screen.findByText('Completed "Buy milk"');
    ds.complete = vi.fn(
      () => new Promise((r) => (release = () => r({ outcome: "applied", op_id: "o", revision: 2, message: "" }))),
    ) as any;
    fireEvent.click(screen.getByLabelText("Complete Call the plumber"));
    const undo = screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement;
    await waitFor(() => expect(undo.disabled).toBe(true));
    release();
    await screen.findByText('Completed "Call the plumber"');
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("checkboxes and + are disabled or hidden when offline", () => {
    const { ds, connection } = fake();
    connection.value = "closed";
    render(<App ds={ds} />);
    expect((screen.getByLabelText("Complete Buy milk") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByLabelText("New task")).toBeNull();
  });

  it("offline dims the screen without taking the offline message's styles", () => {
    const { ds, connection } = fake();
    connection.value = "closed";
    const { container } = render(<App ds={ds} />);
    // `.offline` centres the "Can't reach Twydo" box; the screen must not match it.
    expect(container.querySelector("main")!.className).toBe("screen is-offline");
  });

  it("renders markup in a title as literal text", () => {
    const evil = "<img src=x onerror=alert(1)>";
    const { ds } = fake();
    (ds.view("inbox") as any).value = mkView("Inbox", [row("u9", evil)]);
    const { container } = render(<App ds={ds} />);
    expect(screen.getByText(evil)).toBeTruthy();
    expect(container.querySelector("img")).toBeNull();
  });
});

describe("sync warning", () => {
  const failing = {
    attention: true,
    headline: "Sync failing",
    detail: "Cannot reach the server. Next try 14:10.",
    last_success: null,
  };

  it("shows sync warning when attention", () => {
    const { ds } = fake();
    const inbox = ds.view("inbox") as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: failing };
    render(<App ds={ds} />);
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Sync failing");
    expect(alert.textContent).toContain("Cannot reach the server. Next try 14:10.");
  });

  it("no warning when all is well", () => {
    const { ds } = fake();
    const inbox = ds.view("inbox") as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: { ...failing, attention: false } };
    render(<App ds={ds} />);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("Sync failing")).toBeNull();
  });

  it("warning clears on refetch", async () => {
    const { ds } = fake();
    const inbox = ds.view("inbox") as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: failing };
    render(<App ds={ds} />);
    expect(screen.getByRole("alert")).toBeTruthy();
    inbox.value = { ...inbox.value, sync: { attention: false, headline: "Synced" } };
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});
