import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/preact";
import type { TasksFormParams, TasksFormResult } from "./gen/protocol";
import type { DataSource } from "./data";
import { useFormState } from "./formState";
import { answer, vals } from "./formFixtures";

const flush = () => act(async () => {});

describe("useFormState", () => {
  let calls: TasksFormParams[];
  let replies: ((p: TasksFormParams) => Promise<TasksFormResult>)[];
  let ds: DataSource;
  beforeEach(() => {
    vi.useFakeTimers();
    calls = [];
    replies = [];
    ds = {
      checkForm: (p: TasksFormParams) => {
        calls.push(p);
        const r = replies.shift();
        return r ? r(p) : Promise.resolve(answer({}, p.values ?? vals()));
      },
    } as unknown as DataSource;
  });
  afterEach(() => vi.useRealTimers());

  it("checks once on open and adopts starting values for a new task", async () => {
    const { result } = renderHook(() => useFormState(ds, { new: { collection: "c1" } }));
    await flush();
    expect(calls).toHaveLength(1);
    expect(calls[0]!.values).toBeUndefined();
    expect(result.current.values).toEqual(vals());
    expect(result.current.initial).toEqual(vals());
    expect(result.current.dirty).toBe(false);
  });

  it("debounces typing to one check after 250 ms", async () => {
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    act(() => result.current.setField("title", "W"));
    act(() => result.current.setField("title", "Wa"));
    act(() => void vi.advanceTimersByTime(200));
    expect(calls).toHaveLength(1);
    act(() => void vi.advanceTimersByTime(60));
    await flush();
    expect(calls).toHaveLength(2);
    expect(calls[1]!.values!.title).toBe("Wa");
  });

  it("ignores an out-of-date check", async () => {
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    let first!: (a: TasksFormResult) => void;
    replies.push(() => new Promise((r) => (first = r)));
    act(() => result.current.setField("title", "A"));
    act(() => void vi.advanceTimersByTime(250));
    act(() => result.current.setField("title", "AB"));
    act(() => void vi.advanceTimersByTime(250));
    await flush();
    expect(calls).toHaveLength(3);
    expect(result.current.answer?.form.values.title).toBe("AB");
    await act(async () => first(answer({ revision: 9 }, vals({ title: "A" }))));
    expect(result.current.answer?.form.values.title).toBe("AB");
    expect(result.current.answer?.revision).toBe(1);
  });

  it("canSave only with an op", async () => {
    replies.push(() => Promise.resolve(answer({ op: null, error: "title must not be empty" })));
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    expect(result.current.canSave).toBe(false);
    act(() => result.current.setField("title", "x"));
    expect(result.current.canSave).toBe(false);
    act(() => void vi.advanceTimersByTime(250));
    await flush();
    expect(result.current.canSave).toBe(true);
  });

  it("changed elsewhere asks for review before saving", async () => {
    const theirs = vals({ title: "Theirs" });
    replies.push(() =>
      Promise.resolve(
        answer(
          { changed_since: true, rebased: { initial: theirs, overwritten: ["title"], dropped: ["alert"] } },
          vals({ title: "Theirs", notes: "mine" }),
        ),
      ),
    );
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals({ notes: "mine" }) }));
    await flush();
    expect(result.current.changed).toBe(true);
    expect(result.current.canSave).toBe(false);
    act(() => result.current.review());
    await flush();
    expect(calls[1]!.initial).toEqual(theirs);
    expect(result.current.initial).toEqual(theirs);
    expect(result.current.values?.notes).toBe("mine");
    expect(result.current.reviewed).toEqual({ overwritten: ["title"], dropped: ["alert"] });
    expect(result.current.canSave).toBe(true);
  });

  it("dirty after typing, clean after typing back", async () => {
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    expect(result.current.dirty).toBe(false);
    act(() => result.current.setField("title", "Other"));
    expect(result.current.dirty).toBe(true);
    act(() => result.current.setField("title", "Water plants"));
    expect(result.current.dirty).toBe(false);
  });

  it("canSave stays off while a typed change waits for its check", async () => {
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    let early!: (a: TasksFormResult) => void;
    replies.push(() => new Promise((r) => (early = r)));
    act(() => result.current.setField("title", "A"));
    act(() => void vi.advanceTimersByTime(250));
    act(() => result.current.setField("title", "AB"));
    await act(async () => early(answer({}, vals({ title: "A" }))));
    expect(result.current.canSave).toBe(false);
    act(() => void vi.advanceTimersByTime(250));
    await flush();
    expect(result.current.canSave).toBe(true);
    expect(result.current.answer?.form.values.title).toBe("AB");
  });

  it("exposes a failed check and recovers on recheck", async () => {
    replies.push(() => Promise.reject(new Error("down")));
    const { result } = renderHook(() => useFormState(ds, { uid: "u1", initial: vals(), values: vals() }));
    await flush();
    await flush();
    expect(result.current.failed).toBe(true);
    expect(result.current.canSave).toBe(false);
    act(() => result.current.recheck());
    await flush();
    expect(result.current.failed).toBe(false);
    expect(result.current.canSave).toBe(true);
  });
});
