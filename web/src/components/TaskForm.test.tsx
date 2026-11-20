import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import type { DataSource } from "../data";
import type { TasksFormResult, ViewsListResult } from "../gen/protocol";
import { answer, basis, vals } from "../formFixtures";
import { TaskForm } from "./TaskForm";

afterEach(cleanup);

const lists = {
  epoch: "e",
  revision: 1,
  now: "2026-11-17T09:00:00+01:00",
  today: "2026-11-17",
  entries: [
    { view: { view: "list", list: "c1" }, name: "Home", due_count: 0 },
    { view: { view: "list", list: "c2" }, name: "Work", due_count: 0 },
  ],
} as unknown as ViewsListResult;

function setup(ans: TasksFormResult = answer(), save = vi.fn().mockResolvedValue({})) {
  const ds = { checkForm: vi.fn().mockResolvedValue(ans), save } as unknown as DataSource;
  const onSaved = vi.fn();
  const onCancel = vi.fn();
  render(
    <TaskForm
      ds={ds}
      open={{ uid: "u1", initial: vals(), values: vals(), opened: basis() }}
      lists={lists}
      heading="Edit task"
      saveLabel="Save"
      onSaved={onSaved}
      onCancel={onCancel}
    />,
  );
  return { ds, save, onSaved, onCancel };
}

describe("TaskForm", () => {
  it("renders the fields in the answer's order", async () => {
    const a = answer();
    a.form.fields = ["notes", "title", "due"];
    setup(a);
    await screen.findByLabelText("Title");
    const labels = Array.from(document.querySelectorAll("input[aria-label], textarea")).map((e) => e.getAttribute("aria-label"));
    expect(labels.filter((l) => ["Notes", "Title", "Due"].includes(l!))).toEqual(["Notes", "Title", "Due"]);
    expect(screen.queryByLabelText("Repeat")).toBeNull();
  });

  it("shows previews under the fields", async () => {
    const a = answer();
    a.form.previews = [{ field: "due", text: "Fri 21 Nov" }];
    setup(a);
    expect(await screen.findByText("Fri 21 Nov")).toBeTruthy();
  });

  it("alert input is disabled when not editable", async () => {
    const a = answer();
    a.form.alert_editable = false;
    setup(a);
    const alert = (await screen.findByLabelText("Alert")) as HTMLInputElement;
    expect(alert.disabled).toBe(true);
    expect((screen.getByText("15m") as HTMLButtonElement).disabled).toBe(true);
  });

  it("date picker writes the date as text", async () => {
    const { ds } = setup();
    await screen.findByLabelText("Due");
    fireEvent.change(screen.getByLabelText("Due date"), { target: { value: "2026-12-01" } });
    expect((screen.getByLabelText("Due") as HTMLInputElement).value).toBe("2026-12-01");
    await waitFor(() => expect(ds.checkForm).toHaveBeenCalledTimes(2));
  });

  it("cancel with changes asks to discard", async () => {
    const { onCancel } = setup();
    fireEvent.input(await screen.findByLabelText("Title"), { target: { value: "Other" } });
    fireEvent.click(screen.getByText("Cancel"));
    const dlg = screen.getByRole("dialog");
    expect(within(dlg).getByText("Discard changes?")).toBeTruthy();
    expect(onCancel).not.toHaveBeenCalled();
    fireEvent.click(within(dlg).getByText("Discard"));
    expect(onCancel).toHaveBeenCalled();
  });

  it("cancel without changes closes at once", async () => {
    const { onCancel } = setup();
    await screen.findByLabelText("Title");
    fireEvent.click(screen.getByText("Cancel"));
    expect(onCancel).toHaveBeenCalled();
  });

  it("save submits the op and closes", async () => {
    const a = answer();
    const { save, onSaved } = setup(a);
    const btn = (await screen.findByText("Save")) as HTMLButtonElement;
    await waitFor(() => expect(btn.disabled).toBe(false));
    fireEvent.click(btn);
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("Water plants", expect.objectContaining({ title: "Water plants" })));
    expect(save.mock.calls[0]![0]).toEqual(a.op);
    expect(save.mock.calls[0]![1]).toEqual({ epoch: "e", revision: 1 });
    expect(save.mock.calls[0]![2].form).toEqual({ uid: "u1", initial: vals(), values: vals(), opened: basis() });
  });

  it("freezes the form while a save is out", async () => {
    let done!: (v: unknown) => void;
    const { onSaved, onCancel } = setup(answer(), vi.fn(() => new Promise((r) => (done = r))));
    const btn = (await screen.findByText("Save")) as HTMLButtonElement;
    await waitFor(() => expect(btn.disabled).toBe(false));
    fireEvent.click(btn);
    const controls = Array.from(document.querySelectorAll("form input, form textarea, form select, form button"));
    expect(controls.length).toBeGreaterThan(10);
    for (const c of controls) expect((c as HTMLInputElement).disabled, c.outerHTML).toBe(true);
    // Typing that slips through anyway is not taken.
    fireEvent.input(screen.getByLabelText("Notes"), { target: { value: "late typing" } });
    fireEvent.click(screen.getByText("Cancel"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onCancel).not.toHaveBeenCalled();
    await act(async () => done({}));
    expect(onSaved).toHaveBeenCalledWith("Water plants", vals());
  });

  it("a refusal keeps the sheet open with the message", async () => {
    const { onSaved } = setup(answer(), vi.fn().mockRejectedValue(new Error("nope, not now")));
    const btn = (await screen.findByText("Save")) as HTMLButtonElement;
    await waitFor(() => expect(btn.disabled).toBe(false));
    fireEvent.click(btn);
    expect(await screen.findByText("nope, not now")).toBeTruthy();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("a failed check shows a retry", async () => {
    const ds = {
      checkForm: vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue(answer()),
      save: vi.fn(),
    } as unknown as DataSource;
    render(
      <TaskForm ds={ds} open={{ uid: "u1", initial: vals(), values: vals() }} lists={lists} heading="Edit task" saveLabel="Save" onSaved={vi.fn()} onCancel={vi.fn()} />,
    );
    fireEvent.click(await screen.findByText("Retry"));
    await waitFor(() => expect(screen.queryByText("Retry")).toBeNull());
    expect(ds.checkForm).toHaveBeenCalledTimes(2);
  });

  it("a stale refusal re-checks the form", async () => {
    const err = Object.assign(new Error("stale"), { name: "RpcError", data: { kind: "stale" } });
    const { ds } = setup(answer(), vi.fn().mockRejectedValue(err));
    const btn = (await screen.findByText("Save")) as HTMLButtonElement;
    await waitFor(() => expect(btn.disabled).toBe(false));
    fireEvent.click(btn);
    await waitFor(() => expect(ds.checkForm).toHaveBeenCalledTimes(2));
  });
});
