import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { signal } from "@preact/signals";
import type { DataSource, Connection, Refusal, ViewRef } from "../data";
import type { ChildRow, TasksGetResult, ViewsGetResult, ViewsListResult, ViewRow } from "../gen/protocol";
import { taskHash } from "../route";
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

const viewsListResult = (): ViewsListResult => ({
  epoch: "e",
  revision: 1,
  now: "2026-11-17T09:00:00+01:00",
  today: "2026-11-17",
  entries: [
    { view: { view: "inbox" }, name: "Inbox", color: null, open_count: 3, due_count: 0 },
    { view: { view: "today" }, name: "Today", color: null, open_count: 5, due_count: 2 },
    { view: { view: "all" }, name: "All", color: null, open_count: 0, due_count: 0 },
    { heading: "Lists" },
    { view: { view: "list", list: "c9" }, name: "Bills", color: "#ff0000", open_count: 4, due_count: 0 },
  ],
});

const child = (uid: string, title: string, extra: Partial<ChildRow> = {}): ChildRow => ({
  uid,
  title,
  completed: false,
  due_today: false,
  overdue: false,
  pending: false,
  has_note: false,
  child_count: 0,
  kind: "Task",
  ...extra,
});

const taskResult = (
  uid: string,
  title: string,
  extra: Partial<TasksGetResult["task"]> = {},
  children: ChildRow[] = [],
): TasksGetResult => ({
  epoch: "e",
  revision: 1,
  now: "2026-11-17T09:00:00+01:00",
  today: "2026-11-17",
  task: {
    uid,
    title,
    kind: "Task",
    list: { id: "c1", name: "Inbox" },
    overdue: false,
    due_today: false,
    alerts: [],
    warnings: [],
    conflict: false,
    completed: false,
    pending: false,
    ...extra,
  },
  children,
});

let taskGone: (uid: string) => void = () => {};
let gone: (r: ViewRef) => void = () => {};
let refusal: (r: Refusal) => void = () => {};

function fake(over: Partial<Record<string, unknown>> = {}) {
  const inbox = signal<ViewsGetResult | undefined>(
    mkView("Inbox", [row("u1", "Buy milk", { note: "2%" }), row("u2", "Call the plumber")]),
  );
  const today = signal<ViewsGetResult | undefined>(mkView("Today", [row("u3", "Pay the invoice")], "2026-11-17"));
  const all = signal<ViewsGetResult | undefined>(mkView("All", [row("u4", "Everything else")]));
  const bills = signal<ViewsGetResult | undefined>(mkView("Bills", [row("u5", "Pay rent")]));
  const tasks = new Map<string, ReturnType<typeof signal<TasksGetResult | undefined>>>();
  const taskSignal = (uid: string) => {
    if (!tasks.has(uid)) tasks.set(uid, signal<TasksGetResult | undefined>(undefined));
    return tasks.get(uid)!;
  };
  taskSignal("u1").value = taskResult("u1", "Buy milk", { note: "2%" });
  const connection = signal<Connection>("open");
  const fresh = signal(true);
  const ds = {
    view: (n: ViewRef) =>
      n.view === "today" ? today : n.view === "all" ? all : n.view === "list" ? bills : inbox,
    task: (uid: string) => taskSignal(uid),
    onTaskGone: vi.fn((cb: (uid: string) => void) => {
      taskGone = cb;
      return () => {};
    }),
    reopen: vi.fn(async () => ({ outcome: "applied", op_id: "o", revision: 2, message: "ok" })),
    viewsList: signal(viewsListResult()),
    onGone: vi.fn((cb: (r: ViewRef) => void) => {
      gone = cb;
      return () => {};
    }),
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
  } as unknown as DataSource & { complete: any; create: any; undo: any; reopen: any; refresh: any };
  return { ds, connection, fresh, inbox, bills, taskSignal };
}

beforeEach(() => {
  localStorage.clear();
  history.replaceState(null, "", "/");
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("tabs", () => {
  it("tabs are Inbox, Today then Views with aria-labels", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.getAttribute("aria-label"))).toEqual(["Inbox", "Today", "Views"]);
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

  it("an empty list shows its icon, Nothing here and how to add", () => {
    const { ds, inbox } = fake();
    inbox.value = mkView("Inbox", []);
    const { container } = render(<App ds={ds} />);
    expect(screen.getByText("Nothing here").className).toBe("empty-title");
    expect(screen.getByText("Tap + to add a new task")).toBeTruthy();
    expect(container.querySelector(".empty svg")).toBeTruthy();
  });

  it("an empty Today shows the calendar with today's day", () => {
    localStorage.setItem("twydo.tab", "today");
    const { ds } = fake();
    (ds.view({ view: "today" }) as any).value = mkView("Today", [], "2026-11-17");
    const { container } = render(<App ds={ds} />);
    expect(container.querySelector(".empty svg")!.textContent).toBe("17");
  });

  it("an empty list offline does not suggest adding", () => {
    const { ds, inbox, connection } = fake();
    inbox.value = mkView("Inbox", []);
    connection.value = "closed";
    render(<App ds={ds} />);
    expect(screen.getByText("Nothing here")).toBeTruthy();
    expect(screen.queryByText("Tap + to add a new task")).toBeNull();
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
    (ds.view({ view: "inbox" }) as any).value = mkView("Inbox", [row("u9", evil)]);
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
    const inbox = ds.view({ view: "inbox" }) as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: failing };
    render(<App ds={ds} />);
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Sync failing");
    expect(alert.textContent).toContain("Cannot reach the server. Next try 14:10.");
  });

  it("no warning when all is well", () => {
    const { ds } = fake();
    const inbox = ds.view({ view: "inbox" }) as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: { ...failing, attention: false } };
    render(<App ds={ds} />);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("Sync failing")).toBeNull();
  });

  it("warning clears on refetch", async () => {
    const { ds } = fake();
    const inbox = ds.view({ view: "inbox" }) as unknown as { value: ViewsGetResult };
    inbox.value = { ...inbox.value, sync: failing };
    render(<App ds={ds} />);
    expect(screen.getByRole("alert")).toBeTruthy();
    inbox.value = { ...inbox.value, sync: { attention: false, headline: "Synced" } };
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});

describe("views tab", () => {
  const openViewsTab = () => fireEvent.click(screen.getByRole("tab", { name: "Views" }));
  const back = () => screen.queryByRole("button", { name: "Views" });

  it("lists headings and one button per entry with its counts", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
    expect(screen.getByText("Lists")).toBeTruthy();
    const today = screen.getByRole("button", { name: /Today/ });
    expect(today.querySelector(".view-due")!.textContent).toBe("2");
    expect(today.querySelector(".view-due")!.getAttribute("aria-label")).toBe("2 due");
    expect(today.querySelector(".view-total")!.textContent).toBe("5");
    expect(today.textContent!.indexOf("2")).toBeLessThan(today.textContent!.indexOf("5"));
    expect(screen.getByRole("button", { name: /Bills/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /All/ }).textContent).toBe("All");
  });

  it("tapping an entry opens the view with a back button", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    fireEvent.click(screen.getByRole("button", { name: /All/ }));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("All");
    expect(screen.getByText("Everything else")).toBeTruthy();
    fireEvent.click(back()!);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
    expect(back()).toBeNull();
  });

  it("the Inbox entry opens inside the Views tab", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    fireEvent.click(screen.getByRole("button", { name: /Inbox/ }));
    expect(screen.getByRole("tab", { name: "Views" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Buy milk")).toBeTruthy();
    expect(back()).toBeTruthy();
  });

  it("tapping the Views tab again pops to the list", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    fireEvent.click(screen.getByRole("button", { name: /All/ }));
    openViewsTab();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
  });

  it("an open view survives a tab switch and a reload", () => {
    const { ds } = fake();
    const first = render(<App ds={ds} />);
    openViewsTab();
    fireEvent.click(screen.getByRole("button", { name: /All/ }));
    fireEvent.click(screen.getByRole("tab", { name: "Today" }));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Today");
    openViewsTab();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("All");
    first.unmount();
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("All");
  });

  it("dates the tab icon from the views list on a cold reload into it", () => {
    const { ds } = fake();
    localStorage.setItem("twydo.tab", "views");
    (ds.view({ view: "inbox" }) as any).value = undefined;
    (ds.view({ view: "today" }) as any).value = undefined;
    (ds.viewsList as any).value = { ...viewsListResult(), today: "2026-11-23" };
    render(<App ds={ds} />);
    expect(screen.getByLabelText("Today").textContent).toContain("23");
  });

  it("a deleted open list pops back with a banner", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    fireEvent.click(screen.getByRole("button", { name: /Bills/ }));
    expect(localStorage.getItem("twydo.open")).toContain("c9");
    gone({ view: "list", list: "c9" });
    expect(await screen.findByText("That list is gone")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
    expect(localStorage.getItem("twydo.open")).toBeNull();
  });

  it("a remembered list that is gone pops back on the first fetch", async () => {
    localStorage.setItem("twydo.tab", "views");
    localStorage.setItem("twydo.open", JSON.stringify({ view: "list", list: "c9" }));
    const { ds } = fake({
      refresh: vi.fn(async (v: unknown) => {
        if (v !== "views") gone({ view: "list", list: "c9" });
      }),
    });
    render(<App ds={ds} />);
    expect(await screen.findByText("That list is gone")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
  });

  it("malformed remembered state counts as absent", () => {
    localStorage.setItem("twydo.tab", "views");
    localStorage.setItem("twydo.open", "{nope");
    const { ds } = fake();
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
  });

  it("has no New task button on the list, but has one in an opened view", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openViewsTab();
    expect(screen.queryByLabelText("New task")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /All/ }));
    expect(screen.getByLabelText("New task")).toBeTruthy();
  });

  it("shows the spinner while the views list first loads", () => {
    const { ds } = fake({ viewsList: signal(undefined) });
    render(<App ds={ds} />);
    openViewsTab();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
    expect(screen.getByRole("status", { name: "Loading" })).toBeTruthy();
  });

  it("titles an opened view from the list until its first fetch", () => {
    localStorage.setItem("twydo.tab", "views");
    localStorage.setItem("twydo.open", JSON.stringify({ view: "list", list: "c9" }));
    const { ds, bills } = fake();
    bills.value = undefined;
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Bills");
  });

  it("keeps the title's height when nothing names the view yet", () => {
    localStorage.setItem("twydo.tab", "views");
    localStorage.setItem("twydo.open", JSON.stringify({ view: "list", list: "zz" }));
    const { ds, bills } = fake({ viewsList: signal(undefined) });
    (bills as any).value = undefined;
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("\u00a0");
  });

  it.each([
    [{ view: "bogus" }],
    [{ view: "list" }],
    [{ view: "all", list: "c9" }],
    [{ view: "list", list: 5 }],
  ])("remembered %j is treated as absent", (stored) => {
    localStorage.setItem("twydo.tab", "views");
    localStorage.setItem("twydo.open", JSON.stringify(stored));
    const { ds } = fake();
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Views");
    expect(screen.getByText("Lists")).toBeTruthy();
  });
});

describe("task screen", () => {
  const openU1 = () => fireEvent.click(screen.getByText("Buy milk"));

  it("tapping a row pushes the task screen", () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openU1();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Buy milk");
    expect(location.hash).toBe("#/task/u1");
    expect(screen.getByRole("button", { name: "Inbox" })).toBeTruthy();
    expect(screen.queryByLabelText("New task")).toBeNull();
    expect(ds.refresh).toHaveBeenLastCalledWith({ task: "u1" });
  });

  it("shows every detail field", () => {
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {
      kind: "Project",
      due: "Today 17:00",
      repeat: "Every week",
      alerts: ["At due time", "10 minutes before"],
      start: "Yesterday",
      parent: { uid: "p1", title: "Groceries", kind: "Project" },
      warnings: ["Unknown repeat rule"],
      blocked: "Edits are off",
      conflict: true,
      list: { id: "c1", name: "Inbox", color: "#ff0000" },
      note: "See https://example.com/milk.\nthanks",
    });
    render(<App ds={ds} />);
    openU1();
    expect(screen.getByText("Project \u00b7 Inbox")).toBeTruthy();
    for (const t of ["Due", "Repeat", "Alert", "Start", "In"]) expect(screen.getByText(t)).toBeTruthy();
    for (const t of ["Today 17:00", "Every week", "At due time", "10 minutes before", "Yesterday"]) {
      expect(screen.getByText(t)).toBeTruthy();
    }
    expect(screen.getByRole("button", { name: "Groceries" })).toBeTruthy();
    expect(screen.getByText("! Unknown repeat rule")).toBeTruthy();
    expect(screen.getByText("! Edits are off")).toBeTruthy();
    expect(screen.getByText("This task has a sync conflict. Resolve it in tdo.")).toBeTruthy();
    const link = screen.getByRole("link", { name: "https://example.com/milk" });
    expect(link.getAttribute("href")).toBe("https://example.com/milk");
    expect(link.getAttribute("rel")).toBe("noopener");
  });

  it("ticks and unticks children", async () => {
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim"), child("c2", "Whole", { completed: true })]);
    render(<App ds={ds} />);
    openU1();
    expect(screen.getByText("Items \u00b7 1 open")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Complete Skim"));
    expect(ds.complete).toHaveBeenCalledWith("c1");
    expect(await screen.findByText('Completed "Skim"')).toBeTruthy();
    const done = screen.getByLabelText("Reopen Whole") as HTMLButtonElement;
    expect(done.disabled).toBe(false);
    fireEvent.click(done);
    expect(ds.reopen).toHaveBeenCalledWith("c2");
    expect(await screen.findByText('Reopened "Whole"')).toBeTruthy();
  });

  it("a refused child toggle rolls back", async () => {
    const { ds, taskSignal } = fake();
    ds.complete.mockRejectedValueOnce(new Error("Refused: stale"));
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim")]);
    render(<App ds={ds} />);
    openU1();
    fireEvent.click(screen.getByLabelText("Complete Skim"));
    expect(await screen.findByText("Refused: stale")).toBeTruthy();
    expect(screen.getByLabelText("Complete Skim").getAttribute("aria-checked")).toBe("false");
  });

  it("opening a child pushes its screen; back returns to the parent", async () => {
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim")]);
    taskSignal("c1").value = taskResult("c1", "Skim", { parent: { uid: "u1", title: "Buy milk", kind: "Task" } });
    render(<App ds={ds} />);
    openU1();
    fireEvent.click(screen.getByText("Skim"));
    expect(location.hash).toBe("#/task/c1");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Skim");
    // The back button is labelled with the parent's title, as is the "In" link.
    expect(document.querySelector(".back")!.textContent).toBe("Buy milk");
    fireEvent.click(document.querySelector(".back")!);
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Buy milk"));
    await waitFor(() => expect(location.hash).toBe("#/task/u1"));
  });

  it("reload on a task route shows it; back goes to the tab root", async () => {
    history.replaceState(null, "", taskHash("u1"));
    const { ds } = fake();
    render(<App ds={ds} />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Buy milk");
    fireEvent.click(screen.getByRole("button", { name: "Inbox" }));
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Inbox"));
    expect(location.hash).toBe("");
  });

  it("pops a task that is gone", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openU1();
    taskGone("u2");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Buy milk");
    taskGone("u1");
    expect(await screen.findByText("That task is gone")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Inbox");
    expect(ds.refresh).toHaveBeenLastCalledWith({ view: "inbox" });
  });

  it("offline disables the task screen", () => {
    const { ds, connection, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim"), child("c2", "Whole", { completed: true })]);
    render(<App ds={ds} />);
    openU1();
    connection.value = "closed";
    return waitFor(() => {
      const boxes = screen.getAllByRole("checkbox") as HTMLButtonElement[];
      expect(boxes).toHaveLength(3);
      expect(boxes.every((b) => b.disabled)).toBe(true);
    });
  });

  it("offline, a task never fetched does not open; a cached one does", async () => {
    const { ds, connection, taskSignal } = fake();
    render(<App ds={ds} />);
    connection.value = "closed";
    await waitFor(() => expect(screen.getByText(/reach/i)).toBeTruthy());
    expect(taskSignal("u2").value).toBeUndefined();
    fireEvent.click(screen.getByText("Call the plumber"));
    expect(location.hash).toBe("");
    expect(screen.queryByRole("button", { name: "Inbox" })).toBeNull();
    fireEvent.click(screen.getByText("Buy milk"));
    expect(location.hash).toBe("#/task/u1");
  });

  it("offline, a child never fetched does not open", async () => {
    const { ds, connection, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim")]);
    render(<App ds={ds} />);
    fireEvent.click(screen.getByText("Buy milk"));
    connection.value = "closed";
    await waitFor(() => expect((screen.getAllByRole("checkbox")[0] as HTMLButtonElement).disabled).toBe(true));
    fireEvent.click(screen.getByText("Skim"));
    expect(location.hash).toBe("#/task/u1");
  });

  it("reload on a child screen goes back to the tab root, never past the app", async () => {
    // Saved history state is not trusted on reload: entries before it may not be the app's.
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {}, [child("c1", "Skim")]);
    taskSignal("c1").value = taskResult("c1", "Skim", { parent: { uid: "u1", title: "Buy milk", kind: "Task" } });
    history.replaceState({ stack: ["u1", "c1"] }, "", taskHash("c1"));
    render(<App ds={ds} />);
    expect(document.querySelector(".back")!.textContent).toBe("Inbox");
    fireEvent.click(document.querySelector(".back")!);
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Inbox"));
    expect(location.hash).toBe("");
  });

  it("links in a note keep a balanced closing paren and lose a sentence one", () => {
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", {
      note: "see (https://example.com/a) and https://example.com/Foo_(bar).",
    });
    render(<App ds={ds} />);
    fireEvent.click(screen.getByText("Buy milk"));
    const hrefs = [...document.querySelectorAll(".note a")].map((a) => a.getAttribute("href"));
    expect(hrefs).toEqual(["https://example.com/a", "https://example.com/Foo_(bar)"]);
  });

  it("identical warnings both show", () => {
    const { ds, taskSignal } = fake();
    taskSignal("u1").value = taskResult("u1", "Buy milk", { warnings: ["odd", "odd"] });
    render(<App ds={ds} />);
    fireEvent.click(screen.getByText("Buy milk"));
    expect(screen.getAllByText("! odd")).toHaveLength(2);
  });

  it("tapping the active tab pops to the root", async () => {
    const { ds } = fake();
    render(<App ds={ds} />);
    openU1();
    fireEvent.click(screen.getByLabelText("Inbox", { selector: '[role="tab"]' }));
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toContain("Inbox"));
    await waitFor(() => expect(location.hash).toBe(""));
  });
});
