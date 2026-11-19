import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDataSource, newRequestId, viewKey, type DataSource } from "./data";
import { asWs, FakeRpcError, FakeWebSocket } from "./testutil";

const view = (title: string, revision = 1) => ({
  epoch: "e1",
  revision,
  now: "2026-11-17T09:00:00+01:00",
  today: "2026-11-17",
  title,
  open_count: 0,
  sections: [],
  capture: { collection: "c1", name: "Inbox", due: null },
});

let revision = 1;
let session = 200;
const fetchFn = vi.fn(async () =>
  session === 200
    ? new Response(JSON.stringify({ device: "phone", role: "write" }), { status: 200 })
    : new Response("", { status: 401 }),
);

const made: DataSource[] = [];
function setup() {
  FakeWebSocket.handlers = {
    "events.subscribe": () => ({}),
    "views.get": (p) => view(p.view === "today" ? "Today" : p.view === "list" ? "List" : "Inbox", revision),
    "views.list": () => ({ epoch: "e1", revision, now: "2026-11-17T09:00:00+01:00", today: "2026-11-17", entries: [{ heading: "Lists" }] }),
    "op.submit": () => ({ outcome: "Applied", op_id: "o1", revision: 2, message: "ok" }),
    "op.undo": () => ({ outcome: "Applied", op_id: "o2", revision: 3, message: "ok" }),
  };
  const ds = createDataSource({
    url: "ws://x/ws",
    fetch: fetchFn as unknown as typeof fetch,
    ws: asWs,
  });
  made.push(ds);
  return ds;
}
const flush = () => vi.advanceTimersByTimeAsync(0);
const opened = async () => {
  await flush(); // /session
  FakeWebSocket.last.open();
  await flush();
};
const viewsGets = () => FakeWebSocket.instances.flatMap((w) => w.calls("views.get"));

beforeEach(() => {
  vi.useFakeTimers();
  FakeWebSocket.reset();
  fetchFn.mockClear();
  revision = 1;
  session = 200;
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
});
afterEach(() => {
  while (made.length) made.pop()!.dispose();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
const beats = async (ms: number) => {
  for (let t = 0; t < ms; t += 15000) {
    FakeWebSocket.last.notify("heartbeat", { seq: t });
    await vi.advanceTimersByTimeAsync(15000);
  }
};

describe("data source", () => {
  it("is unpaired when /session is 401", async () => {
    session = 401;
    const ds = setup();
    await flush();
    expect(ds.connection.value).toBe("unpaired");
    // stays unpaired: no reconnect storm
    await vi.advanceTimersByTimeAsync(60000);
    expect(ds.connection.value).toBe("unpaired");
  });

  it("subscribes and fetches on open", async () => {
    const ds = setup();
    expect(ds.connection.value).toBe("connecting");
    await opened();
    expect(ds.connection.value).toBe("open");
    const ws = FakeWebSocket.last;
    expect(ws.calls("events.subscribe")).toHaveLength(1);
    expect(ws.calls("views.get")[0]!.params).toEqual({ view: "inbox" });
    expect(ds.view({ view: "inbox" }).value?.title).toBe("Inbox");
    expect(ds.view({ view: "today" }).value).toBeUndefined();
  });

  it("refetches on data.changed, status.changed and missed", async () => {
    setup();
    await opened();
    const n = viewsGets().length;
    revision = 2;
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 1, revision: 2, cause: "sync", uids: [] });
    await flush();
    expect(viewsGets()).toHaveLength(n + 1);
    FakeWebSocket.last.notify("event", { type: "status.changed", seq: 2, status_rev: 3 });
    await flush();
    expect(viewsGets()).toHaveLength(n + 2);
    FakeWebSocket.last.notify("missed", { seq: 3, revision: 2, status_rev: 3 });
    await flush();
    expect(viewsGets()).toHaveLength(n + 3);
    FakeWebSocket.last.notify("event", { type: "op.finished", seq: 4 });
    FakeWebSocket.last.notify("heartbeat", { seq: 5 });
    await flush();
    expect(viewsGets()).toHaveLength(n + 3);
  });

  it("refresh fetches the named view and makes it the visible one", async () => {
    const ds = setup();
    await opened();
    await ds.refresh({ view: "today" });
    expect(ds.view({ view: "today" }).value?.title).toBe("Today");
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 1 });
    await flush();
    expect(viewsGets().at(-1)!.params).toEqual({ view: "today" });
  });

  it("visibility change reconnects and refetches", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.last.drop(1006, "");
    await flush();
    expect(ds.connection.value).toBe("closed");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.last.open();
    await flush();
    expect(FakeWebSocket.last.calls("views.get")).toHaveLength(1);
    // already open: a visibility change just refetches
    document.dispatchEvent(new Event("visibilitychange"));
    await flush();
    expect(FakeWebSocket.last.calls("views.get")).toHaveLength(2);
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it("periodic refetch every five minutes while visible", async () => {
    setup();
    await opened();
    const n = viewsGets().length;
    await beats(5 * 60 * 1000);
    expect(viewsGets()).toHaveLength(n + 1);
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    await beats(5 * 60 * 1000);
    expect(viewsGets()).toHaveLength(n + 1);
  });

  it("complete sends based_on from the last view", async () => {
    const ds = setup();
    await opened();
    revision = 7;
    await ds.refresh({ view: "inbox" });
    const r = await ds.complete("uid-1");
    expect(r.outcome).toBe("Applied");
    const m = FakeWebSocket.last.calls("op.submit")[0]!;
    expect(m.params).toMatchObject({
      based_on: { epoch: "e1", revision: 7 },
      op: { Complete: { uid: "uid-1" } },
    });
    expect(typeof m.params.request_id).toBe("string");
    expect(typeof m.params.session).toBe("string");
  });

  it("a refused op.finished for a queued completion reaches onRefusal with its uid", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.handlers["op.submit"] = () => ({ outcome: "queued", op_id: "o9", revision: 2, message: "q" });
    const seen: unknown[] = [];
    ds.onRefusal((r) => seen.push(r));
    await ds.complete("uid-1");
    const rid = FakeWebSocket.last.calls("op.submit")[0]!.params.request_id;
    FakeWebSocket.last.notify("event", {
      type: "op.finished",
      seq: 9,
      op_id: "o9",
      request_id: rid,
      state: { state: "applied", op_id: "o9" },
    });
    expect(seen).toEqual([]);
    await ds.complete("uid-2");
    const rid2 = FakeWebSocket.last.calls("op.submit")[1]!.params.request_id;
    FakeWebSocket.last.notify("event", {
      type: "op.finished",
      seq: 10,
      op_id: "o10",
      request_id: rid2,
      state: { state: "refused", kind: "stale", message: "The task changed" },
    });
    expect(seen).toEqual([{ kind: "complete", uid: "uid-2", message: "The task changed" }]);
  });

  it("a refused op.finished for a queued capture reaches onRefusal with its draft", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.handlers["op.submit"] = () => ({ outcome: "queued", op_id: "o9", revision: 2, message: "q" });
    const seen: unknown[] = [];
    ds.onRefusal((r) => seen.push(r));
    const capture = { collection: "c1", name: "Inbox", due: null };
    await ds.create("Water plants", "ferns", capture);
    const rid = FakeWebSocket.last.calls("op.submit")[0]!.params.request_id;
    const finished = { type: "op.finished", seq: 9, op_id: "o9", request_id: rid };
    FakeWebSocket.last.notify("event", { ...finished, state: { state: "refused", kind: "stale", message: "nope" } });
    expect(seen).toEqual([{ kind: "create", title: "Water plants", note: "ferns", capture, message: "nope" }]);
    // The entry is consumed: a repeat reports nothing.
    FakeWebSocket.last.notify("event", { ...finished, state: { state: "refused", message: "nope" } });
    expect(seen).toHaveLength(1);
  });

  it("a cancelled op.finished for a queued undo reaches onRefusal", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.handlers["op.undo"] = () => ({ outcome: "queued", op_id: "o9", revision: 2, message: "q" });
    const seen: unknown[] = [];
    ds.onRefusal((r) => seen.push(r));
    await ds.undo();
    const rid = FakeWebSocket.last.calls("op.undo")[0]!.params.request_id;
    FakeWebSocket.last.notify("event", {
      type: "op.finished",
      seq: 9,
      op_id: "o9",
      request_id: rid,
      state: { state: "cancelled", message: "gone" },
    });
    expect(seen).toEqual([{ kind: "undo", message: "gone" }]);
  });

  it("an applied (non-queued) capture is not tracked", async () => {
    const ds = setup();
    await opened();
    const seen: unknown[] = [];
    ds.onRefusal((r) => seen.push(r));
    await ds.create("Buy milk", "", { collection: "c1", name: "Inbox", due: null });
    const rid = FakeWebSocket.last.calls("op.submit")[0]!.params.request_id;
    FakeWebSocket.last.notify("event", {
      type: "op.finished",
      seq: 9,
      op_id: "o",
      request_id: rid,
      state: { state: "refused", message: "late" },
    });
    expect(seen).toEqual([]);
  });

  it("retry reuses request_id", async () => {
    const ds = setup();
    await opened();
    delete FakeWebSocket.handlers["op.submit"]; // the first call never gets a reply
    const first = ds.complete("uid-1");
    await flush();
    FakeWebSocket.last.drop(1006, "");
    await expect(first).rejects.toThrow();
    FakeWebSocket.handlers["op.submit"] = () => ({ outcome: "Applied", op_id: "o", revision: 2, message: "" });
    await vi.advanceTimersByTimeAsync(1000);
    FakeWebSocket.last.open();
    await flush();
    await ds.complete("uid-1");
    const ids = FakeWebSocket.instances.flatMap((w) => w.calls("op.submit")).map((m) => m.params.request_id);
    expect(ids).toHaveLength(2);
    expect(ids[0]).toBe(ids[1]);
    // a different action gets a new id
    await ds.complete("uid-2");
    const last = FakeWebSocket.last.calls("op.submit").at(-1)!;
    expect(last.params.request_id).not.toBe(ids[0]);
  });

  it("stale refusal refetches", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.handlers["op.submit"] = () => {
      throw new FakeRpcError(-32000, "stale", "stale");
    };
    const n = viewsGets().length;
    revision = 9;
    await expect(ds.complete("uid-1")).rejects.toMatchObject({ data: { kind: "stale" } });
    await flush();
    expect(viewsGets()).toHaveLength(n + 1);
    expect(ds.view({ view: "inbox" }).value?.revision).toBe(9);
  });

  it("create in Inbox sends no due date", async () => {
    const ds = setup();
    await opened();
    await ds.create("Buy milk", "", { collection: "c1", name: "Inbox", due: null });
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.op).toEqual({
      Create: { collection: "c1", summary: "Buy milk", description: null, kind: "Task", due: null },
    });
  });

  it("today capture sends due date", async () => {
    const ds = setup();
    await opened();
    await ds.create("Call back", "re: invoice", { collection: "c1", name: "Inbox", due: "2026-11-17" });
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.op).toEqual({
      Create: {
        collection: "c1",
        summary: "Call back",
        description: "re: invoice",
        kind: "Task",
        due: { Date: "2026-11-17" },
      },
    });
  });

  it("undo sends op.undo", async () => {
    const ds = setup();
    await opened();
    const r = await ds.undo();
    expect(r.op_id).toBe("o2");
    expect(FakeWebSocket.last.calls("op.undo")[0]!.params).toMatchObject({ request_id: expect.any(String) });
  });

  it("a write issued while connecting waits for open and a fresh view", async () => {
    const ds = setup();
    await opened();
    revision = 4;
    FakeWebSocket.last.drop(1006, "");
    await flush();
    await vi.advanceTimersByTimeAsync(1000); // reconnect attempt: connecting
    expect(ds.connection.value).toBe("connecting");
    const p = ds.complete("uid-1");
    await flush();
    expect(FakeWebSocket.last.calls("op.submit")).toHaveLength(0);
    FakeWebSocket.last.open();
    await flush();
    await p;
    // The displayed view (revision 1) is the basis; the daemon decides whether it is stale.
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.based_on.revision).toBe(1);
  });

  it("a write with no displayed view fetches first and uses that revision", async () => {
    const ds = setup();
    revision = 5;
    await flush(); // /session
    const p = ds.complete("uid-1");
    FakeWebSocket.last.open();
    await flush();
    await p;
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.based_on.revision).toBe(5);
  });

  it("a tick on a cached tab sends that tab's revision, not another view's", async () => {
    const ds = setup();
    await opened(); // inbox at 1
    revision = 1;
    await ds.refresh({ view: "today" }); // today cached at 1
    revision = 2;
    await ds.refresh({ view: "inbox" }); // inbox fetched at 2
    // Back to Today with its refresh still pending.
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    FakeWebSocket.handlers["views.get"] = (p) =>
      view(p.view === "today" ? "Today" : "Inbox", revision);
    const orig = FakeWebSocket.last.send.bind(FakeWebSocket.last);
    let hold = true;
    FakeWebSocket.last.send = (d: string) => {
      const m = JSON.parse(d);
      if (hold && m.method === "views.get") {
        void gate.then(() => orig(d));
        return;
      }
      orig(d);
    };
    const refreshing = ds.refresh({ view: "today" });
    await flush();
    expect(ds.view({ view: "today" }).value?.revision).toBe(1);
    const p = ds.complete("uid-1");
    await flush();
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.based_on.revision).toBe(1);
    hold = false;
    release();
    await refreshing;
    await p;
  });

  it("switching to a cached view is not fresh until its fetch completes", async () => {
    const ds = setup();
    await opened();
    await ds.refresh({ view: "today" });
    expect(ds.fresh.value).toBe(true);
    const orig = FakeWebSocket.last.send.bind(FakeWebSocket.last);
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    FakeWebSocket.last.send = (d: string) => {
      if (JSON.parse(d).method === "views.get") void gate.then(() => orig(d));
      else orig(d);
    };
    const p = ds.refresh({ view: "inbox" });
    expect(ds.fresh.value).toBe(false);
    await flush();
    expect(ds.fresh.value).toBe(false);
    release();
    await p;
    expect(ds.fresh.value).toBe(true);
  });

  it("another view's fetch finishing does not make the visible view fresh", async () => {
    const ds = setup();
    await opened(); // inbox visible and fresh
    const orig = FakeWebSocket.last.send.bind(FakeWebSocket.last);
    const held: string[] = [];
    FakeWebSocket.last.send = (d: string) => {
      if (JSON.parse(d).method === "views.get") held.push(d);
      else orig(d);
    };
    const first = ds.refresh({ view: "today" }); // in flight, held
    await flush();
    const second = ds.refresh({ view: "inbox" }); // visible is inbox again; coalesced behind the first
    expect(ds.fresh.value).toBe(false);
    orig(held[0]!); // today's reply arrives while inbox is visible
    await flush();
    expect(ds.view({ view: "today" }).value).toBeDefined();
    expect(ds.fresh.value).toBe(false);
    await flush();
    orig(held[1]!); // the visible view's own fetch completes
    await Promise.all([first, second]);
    expect(ds.fresh.value).toBe(true);
  });

  it("a write gives up when the socket does not open", async () => {
    const ds = setup();
    await opened();
    FakeWebSocket.last.drop(1006, "");
    await flush();
    const p = ds.complete("uid-1");
    p.catch(() => {});
    await vi.advanceTimersByTimeAsync(5000);
    await expect(p).rejects.toThrow();
  });

  it("a silent socket times out, reconnects and recovers", async () => {
    const ds = setup();
    delete FakeWebSocket.handlers["views.get"];
    await opened(); // views.get never answers
    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.handlers["views.get"] = () => view("Inbox", 3);
    await vi.advanceTimersByTimeAsync(10000);
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.last.open();
    await flush();
    expect(ds.view({ view: "inbox" }).value?.revision).toBe(3);
    expect(ds.fresh.value).toBe(true);
  });

  it("a socket with no heartbeat is replaced", async () => {
    setup();
    await opened();
    await vi.advanceTimersByTimeAsync(46000);
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it("visibility on an open but dead socket reconnects", async () => {
    setup();
    await opened();
    delete FakeWebSocket.handlers["views.get"];
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(10000);
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it("fresh is true only after a fetch on an open socket", async () => {
    const ds = setup();
    expect(ds.fresh.value).toBe(false);
    await flush();
    FakeWebSocket.last.open();
    expect(ds.fresh.value).toBe(false);
    await flush();
    expect(ds.fresh.value).toBe(true);
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(ds.fresh.value).toBe(false);
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(ds.fresh.value).toBe(false);
    await flush();
    expect(ds.fresh.value).toBe(true);
    FakeWebSocket.last.drop(1006, "");
    expect(ds.fresh.value).toBe(false);
  });

  it("falls back to getRandomValues when randomUUID is missing", async () => {
    const real = globalThis.crypto;
    vi.stubGlobal("crypto", { getRandomValues: (a: Uint8Array<ArrayBuffer>) => real.getRandomValues(a) });
    expect(newRequestId()).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    const ds = setup();
    await opened();
    await ds.complete("uid-1");
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.request_id).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("separate undos send different request ids", async () => {
    const ds = setup();
    await opened();
    await ds.undo();
    await ds.undo();
    const ids = FakeWebSocket.last.calls("op.undo").map((m) => m.params.request_id);
    expect(ids[0]).not.toBe(ids[1]);
  });

  it("an id kept after a lost connection is dropped once the retry window passes", async () => {
    const ds = setup();
    await opened();
    delete FakeWebSocket.handlers["op.undo"];
    const first = ds.undo();
    first.catch(() => {});
    await flush();
    FakeWebSocket.last.drop(1006, "");
    await expect(first).rejects.toThrow();
    FakeWebSocket.handlers["op.undo"] = () => ({ outcome: "Applied", op_id: "o", revision: 2, message: "" });
    await beats(0);
    await vi.advanceTimersByTimeAsync(61000);
    FakeWebSocket.last.open();
    await flush();
    await ds.undo();
    const ids = FakeWebSocket.instances.flatMap((w) => w.calls("op.undo")).map((m) => m.params.request_id);
    expect(ids).toHaveLength(2);
    expect(ids[0]).not.toBe(ids[1]);
  });

  it("starting a different action drops the kept id", async () => {
    const ds = setup();
    await opened();
    delete FakeWebSocket.handlers["op.submit"];
    const first = ds.complete("a");
    first.catch(() => {});
    await flush();
    FakeWebSocket.last.drop(1006, "");
    await expect(first).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(1000);
    FakeWebSocket.last.open();
    await flush();
    FakeWebSocket.handlers["op.submit"] = () => ({ outcome: "Applied", op_id: "o", revision: 2, message: "" });
    await ds.complete("b");
    await ds.complete("a");
    const ids = FakeWebSocket.instances.flatMap((w) => w.calls("op.submit")).map((m) => m.params.request_id);
    expect(new Set(ids).size).toBe(3);
  });
});

describe("all views", () => {
  const viewsLists = () => FakeWebSocket.instances.flatMap((w) => w.calls("views.list"));

  it("view_key_forms", () => {
    expect(viewKey({ view: "list", list: "w" })).toBe("list:w");
    expect(viewKey({ view: "done" })).toBe("done");
  });

  it("refresh_views_fetches_views_list", async () => {
    const ds = setup();
    await opened();
    await ds.refresh("views");
    expect(viewsLists().at(-1)!.params).toEqual({});
    expect(ds.viewsList.value?.entries).toEqual([{ heading: "Lists" }]);
  });

  it("refresh_list_sends_list_param", async () => {
    const ds = setup();
    await opened();
    const before = ds.view({ view: "inbox" }).value;
    await ds.refresh({ view: "list", list: "w" });
    expect(viewsGets().at(-1)!.params).toEqual({ view: "list", list: "w" });
    expect(ds.view({ view: "list", list: "w" }).value?.title).toBe("List");
    expect(ds.view({ view: "inbox" }).value).toBe(before);
  });

  it("data_changed_refetches_visible", async () => {
    const ds = setup();
    await opened();
    await ds.refresh("views");
    const gets = viewsGets().length;
    const lists = viewsLists().length;
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 1 });
    await flush();
    expect(viewsLists()).toHaveLength(lists + 1);
    expect(viewsGets()).toHaveLength(gets);
  });

  it("gone_list_reports_once", async () => {
    const ds = setup();
    await opened();
    const ref = { view: "list" as const, list: "w" };
    await ds.refresh(ref);
    expect(ds.view(ref).value).toBeDefined();
    const gone = vi.fn();
    ds.onGone(gone);
    FakeWebSocket.handlers["views.get"] = () => {
      throw new FakeRpcError(-32602, "no such list", "invalid");
    };
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 1 }); // a quiet refetch
    await flush();
    expect(gone).toHaveBeenCalledTimes(1);
    expect(gone).toHaveBeenCalledWith(ref);
    expect(ds.view(ref).value).toBeUndefined();
    // Visible is now the views list: a later refetch asks for it and reports nothing more.
    const lists = viewsLists().length;
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 2 });
    await flush();
    expect(viewsLists()).toHaveLength(lists + 1);
    expect(gone).toHaveBeenCalledTimes(1);
  });

  it("gone_list_not_shown_is_not_reported_and_keeps_the_coalesced_fetch", async () => {
    const ds = setup();
    await opened();
    const ref = { view: "list" as const, list: "w" };
    await ds.refresh(ref);
    const gone = vi.fn();
    ds.onGone(gone);
    const orig = FakeWebSocket.last.send.bind(FakeWebSocket.last);
    const held: string[] = [];
    FakeWebSocket.last.send = (d: string) => {
      if (JSON.parse(d).method === "views.get") held.push(d);
      else orig(d);
    };
    const first = ds.refresh(ref); // in flight, held
    await flush();
    const second = ds.refresh({ view: "today" }); // navigated away; coalesced
    FakeWebSocket.last.fail(JSON.parse(held[0]!).id, -32602, "no such list", "invalid");
    await flush();
    orig(held[1]!);
    await Promise.all([first, second]);
    expect(gone).not.toHaveBeenCalled();
    expect(held).toHaveLength(2);
    expect(JSON.parse(held[1]!).params).toEqual({ view: "today" });
    expect(ds.view({ view: "today" }).value?.title).toBe("Today");
    expect(ds.view(ref).value).toBeUndefined();
  });

  it("fresh_is_false_after_refresh_views_until_views_list_answers", async () => {
    const ds = setup();
    await opened();
    expect(ds.fresh.value).toBe(true);
    const p = ds.refresh("views");
    expect(ds.fresh.value).toBe(false);
    await p;
    expect(ds.fresh.value).toBe(true);
  });

  it("complete_uses_opened_view_basis", async () => {
    const ds = setup();
    await opened(); // inbox at 1
    revision = 3;
    await ds.refresh({ view: "inbox" });
    revision = 7;
    await ds.refresh({ view: "list", list: "w" });
    await ds.complete("uid-1");
    expect(FakeWebSocket.last.calls("op.submit")[0]!.params.based_on.revision).toBe(7);
  });
});
