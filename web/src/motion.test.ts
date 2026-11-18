import { describe, expect, it } from "vitest";
import type { ViewRow, ViewSection } from "./gen/protocol";
import { departures, display, arrivals } from "./motion";

const r = (uid: string): ViewRow => ({
  uid,
  title: uid,
  completed: false,
  conflict: false,
  depth: 0,
  due_today: false,
  overdue: false,
  pending: false,
  repeats: false,
  kind: "Task",
  list: { id: "c1", name: "Inbox" },
});
const sec = (group: ViewSection["group"], ...uids: string[]): ViewSection => ({
  group,
  label: group,
  rows: uids.map(r),
});
const shape = (secs: ReturnType<typeof display>) =>
  secs.map((s) => `${s.group}:${s.rows.map((x) => (x.leaving ? `-${x.row.uid}` : x.row.uid)).join(",")}`);

describe("departures", () => {
  it("lists rows the new view no longer has, with their old neighbour", () => {
    const old = [sec("today", "a", "b", "c")];
    const next = [sec("today", "a", "c")];
    expect(departures(old, next, new Set(["b"]))).toEqual([
      { row: r("b"), group: "today", label: "today", after: "a", afterGroup: null, done: true },
    ]);
  });

  it("a row that moved to another section has not departed", () => {
    expect(departures([sec("today", "a")], [sec("tomorrow", "a")], new Set())).toEqual([]);
  });
});

describe("arrivals", () => {
  it("is empty on the first view", () => {
    expect(arrivals(undefined, [sec("today", "a")])).toEqual(new Set());
  });

  it("names new rows and rows that changed section", () => {
    const old = [sec("today", "a", "b")];
    const next = [sec("today", "a", "c"), sec("tomorrow", "b")];
    expect(arrivals(old, next)).toEqual(new Set(["c", "b"]));
  });
});

describe("display", () => {
  it("keeps a leaving row where it was", () => {
    const old = [sec("today", "a", "b", "c")];
    const next = [sec("today", "a", "c")];
    expect(shape(display(next, departures(old, next, new Set())))).toEqual(["today:a,-b,c"]);
  });

  it("keeps a leaving first row first", () => {
    const old = [sec("today", "a", "b")];
    const next = [sec("today", "b")];
    expect(shape(display(next, departures(old, next, new Set())))).toEqual(["today:-a,b"]);
  });

  it("keeps an emptied section in its old place until its rows have left", () => {
    const old = [sec("overdue", "a"), sec("today", "b"), sec("tomorrow", "c")];
    const next = [sec("overdue", "a"), sec("tomorrow", "c")];
    expect(shape(display(next, departures(old, next, new Set())))).toEqual([
      "overdue:a",
      "today:-b",
      "tomorrow:c",
    ]);
  });

  it("keeps consecutive leaving rows in order", () => {
    const old = [sec("today", "a", "b", "c", "d")];
    const next = [sec("today", "a", "d")];
    expect(shape(display(next, departures(old, next, new Set())))).toEqual(["today:a,-b,-c,d"]);
  });

  it("puts an emptied first section first", () => {
    const old = [sec("overdue", "a"), sec("today", "b")];
    const next = [sec("today", "b")];
    expect(shape(display(next, departures(old, next, new Set())))).toEqual(["overdue:-a", "today:b"]);
  });
});
