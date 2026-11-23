// The only module screens use to reach the daemon (design §9.4).
import { signal, type ReadonlySignal } from "@preact/signals";
import type {
  BasedOn,
  FormBasis,
  FormValues,
  NewTaskTarget,
  Op,
  SettingsGetResult,
  TasksFormParams,
  TasksFormResult,
  TasksGetResult,
  ViewsGetParams,
  ViewsGetResult,
  ViewsListResult,
  WriteResult,
} from "./gen/protocol";
import { RpcSocket } from "./rpc";

// JSON-RPC: the daemon does not have the method.
const METHOD_NOT_FOUND = -32601;

/** One view as `views.get` takes it; a list also names its collection. */
export type ViewRef = ViewsGetParams;
/** One task screen, by task uid. */
export type TaskRef = { task: string };
/** What the screen shows: a view, the Views tab's own list of views, or one task. */
export type Visible = ViewRef | "views" | "settings" | TaskRef;
export const isTask = (v: Visible): v is TaskRef => typeof v === "object" && "task" in v;
export const viewKey = (ref: ViewRef): string => (ref.view === "list" ? `list:${ref.list}` : ref.view);
export type Connection = "connecting" | "open" | "closed" | "unpaired";

const sameTarget = (a: Visible, b: Visible): boolean => {
  if (isTask(a) || isTask(b)) return isTask(a) && isTask(b) && a.task === b.task;
  return typeof a === "string" || typeof b === "string" ? a === b : viewKey(a) === viewKey(b);
};

export interface DataSource {
  view(ref: ViewRef): ReadonlySignal<ViewsGetResult | undefined>;
  /** One task and its children, once fetched. */
  task(uid: string): ReadonlySignal<TasksGetResult | undefined>;
  /** The sidebar entries of the Views tab, once fetched. */
  viewsList: ReadonlySignal<ViewsListResult | undefined>;
  /**
   * Calls `cb` with Today's open count (the icon badge) on every newer views list or Today view,
   * also when the count is unchanged: a push may have changed the badge meanwhile.
   */
  onTodayCount(cb: (n: number) => void): () => void;
  /** The Settings screen's daemon details, once fetched; null when the daemon has no `settings.get`. */
  settings: ReadonlySignal<SettingsGetResult | null | undefined>;
  connection: ReadonlySignal<Connection>;
  /**
   * True only while the visible view was fetched successfully on the current open socket; false
   * when hidden, closed, or after switching to a view whose fetch has not completed.
   */
  fresh: ReadonlySignal<boolean>;
  refresh(v: Visible): Promise<void>;
  /** Fetch the views list into `viewsList` without changing the visible target or `fresh`. */
  loadViewsList(): Promise<void>;
  /**
   * The shown list view was refused as invalid (the list is gone). Its cached entry is dropped
   * and the visible target becomes "views", so this fires once. Fires for quiet refetches too. Returns an unsubscribe function.
   */
  onGone(cb: (ref: ViewRef) => void): () => void;
  /**
   * The visible task's `tasks.get` was refused as invalid (the task is gone). Its signal is
   * cleared and the visible target is left as is; the screen pops it. Returns an unsubscribe
   * function.
   */
  onTaskGone(cb: (uid: string) => void): () => void;
  /** based_on is the epoch/revision of the view displayed when called, never a newer fetch. */
  complete(uid: string): Promise<WriteResult>;
  reopen(uid: string): Promise<WriteResult>;
  undo(): Promise<WriteResult>;
  /** A plain `tasks.form` call: check the values and get the form's previews and fields. */
  checkForm(p: TasksFormParams): Promise<TasksFormResult>;
  /** Submit a task edit or new task on the basis `tasks.form` returned, not the visible view's. */
  save(op: Op, basedOn: BasedOn, what: SaveInfo): Promise<WriteResult>;
  /** Delete a task. */
  remove(uid: string, title: string): Promise<WriteResult>;
  /**
   * A queued write or undo the daemon later refused or cancelled (`op.finished`).
   * Returns an unsubscribe function.
   */
  onRefusal(cb: (refusal: Refusal) => void): () => void;
  /** Stop listening and close the socket. */
  dispose(): void;
}

/** What is needed to reopen a task form: an existing task or a new one, with its values. */
/**
 * What a task form opens with. `opened` is the daemon's opaque `form.basis` from when the form
 * was opened (or last reviewed), sent back with every check.
 */
export type FormOpen = {
  uid?: string;
  new?: NewTaskTarget;
  initial?: FormValues;
  values?: FormValues;
  opened?: FormBasis;
};
/** What a save reports if the daemon later refuses it; no `uid` means a new task. */
export type SaveInfo = { uid?: string; title: string; form: FormOpen };

/** A queued action the daemon later refused or cancelled, with what is needed to report it. */
export type Refusal =
  | { kind: "complete"; uid: string; message: string }
  | { kind: "reopen"; uid: string; message: string }
  | { kind: "add"; title: string; form: FormOpen; message: string }
  | { kind: "save"; uid: string; title: string; form: FormOpen; message: string }
  | { kind: "delete"; uid: string; title: string; message: string }
  | { kind: "undo"; message: string };

type Pending = Refusal extends infer R ? (R extends Refusal ? Omit<R, "message"> : never) : never;

export interface DataSourceOptions {
  url?: string;
  fetch?: typeof fetch;
  ws?: typeof WebSocket;
  now?: () => number;
}

const PERIOD_MS = 5 * 60 * 1000;
// A kept request id is only for retrying the same action; after this it is dropped.
const RETRY_WINDOW_MS = 60 * 1000;
const READY_MS = 5000;

/** A v4 id; `randomUUID` exists only in secure contexts (plain-HTTP LAN setups lack it). */
export function newRequestId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6]! & 0x0f) | 0x40;
  b[8] = (b[8]! & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

function defaultUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws`;
}

export function createDataSource(opts: DataSourceOptions = {}): DataSource {
  const url = opts.url ?? defaultUrl();
  const fetchFn = opts.fetch ?? ((...a: Parameters<typeof fetch>) => fetch(...a));
  const sessionUrl = new URL("/session", url.replace(/^ws/, "http")).toString();

  const views = new Map<string, ReturnType<typeof signal<ViewsGetResult | undefined>>>();
  const viewSignal = (ref: ViewRef) => {
    const key = viewKey(ref);
    let s = views.get(key);
    if (!s) views.set(key, (s = signal<ViewsGetResult | undefined>(undefined)));
    return s;
  };
  const tasks = new Map<string, ReturnType<typeof signal<TasksGetResult | undefined>>>();
  const taskSignal = (uid: string) => {
    let s = tasks.get(uid);
    if (!s) tasks.set(uid, (s = signal<TasksGetResult | undefined>(undefined)));
    return s;
  };
  const taskGoneCbs = new Set<(uid: string) => void>();
  const viewsList = signal<ViewsListResult | undefined>(undefined);
  const countCbs = new Set<(n: number) => void>();
  // Requests run concurrently in the daemon and can answer out of order: each request that
  // carries Today's count takes a ticket when sent, and only a newer ticket's answer is applied.
  let issued = 0;
  let countAt = 0;
  let listAt = 0;
  const ticket = () => ++issued;
  const noteCount = (t: number, n: number) => {
    if (t <= countAt) return;
    countAt = t;
    for (const cb of countCbs) cb(n);
  };
  const setList = (r: ViewsListResult, t: number) => {
    if (t <= listAt) return;
    listAt = t;
    viewsList.value = r;
    for (const e of r.entries) {
      if ("view" in e && e.view.view === "today") noteCount(t, e.open_count);
    }
  };
  const fetchList = async () => {
    const t = ticket();
    setList(await sock.call<ViewsListResult>("views.list", {}), t);
  };
  const settings = signal<SettingsGetResult | null | undefined>(undefined);
  const goneCbs = new Set<(ref: ViewRef) => void>();
  const connection = signal<Connection>("connecting");
  let visible: Visible = { view: "inbox" };
  let unpaired = false;
  const fresh = signal(false);
  const now = opts.now ?? Date.now;
  // The one action that lost its connection before a reply; a retry of it reuses the id.
  let kept: { key: string; id: string; at: number } | undefined;

  // Queued-capable actions in flight, by request id, so a later `op.finished` can name them.
  const pending = new Map<string, Pending>();
  const refusalCbs = new Set<(r: Refusal) => void>();

  const sock = new RpcSocket(url, opts.ws);

  async function checkSession(): Promise<void> {
    try {
      const r = await fetchFn(sessionUrl, { credentials: "same-origin" });
      if (r.status === 401) {
        unpaired = true;
        connection.value = "unpaired";
        sock.close();
      }
    } catch {
      // offline: the socket's own backoff keeps trying
    }
  }

  // Fetches of the visible view coalesce: one in flight, one more if asked meanwhile.
  let fetching: Promise<void> | undefined;
  let again = false;
  function fetchVisible(): Promise<void> {
    if (fetching) {
      again = true;
      return fetching;
    }
    fetching = (async () => {
      try {
        do {
          again = false;
          const target = visible;
          if (target === "views") {
            await fetchList();
          } else if (target === "settings") {
            try {
              settings.value = await sock.call<SettingsGetResult>("settings.get", {});
            } catch (e) {
              // A daemon older than the Settings screen: show what the phone itself knows.
              if ((e as { code?: number }).code !== METHOD_NOT_FOUND) throw e;
              settings.value = null;
            }
          } else if (isTask(target)) {
            try {
              taskSignal(target.task).value = await sock.call<TasksGetResult>("tasks.get", { uid: target.task });
            } catch (e) {
              if ((e as { data?: { kind?: string } }).data?.kind !== "invalid") throw e;
              taskSignal(target.task).value = undefined;
              if (!sameTarget(target, visible)) {
                if (again) continue;
                return;
              }
              for (const cb of taskGoneCbs) cb(target.task);
              if (again) continue;
              throw e;
            }
          } else {
            const params: ViewsGetParams = target.view === "list" ? { view: "list", list: target.list } : { view: target.view };
            let v: ViewsGetResult;
            const t = ticket();
            try {
              v = await sock.call<ViewsGetResult>("views.get", params);
            } catch (e) {
              if (target.view !== "list" || (e as { data?: { kind?: string } }).data?.kind !== "invalid") throw e;
              viewSignal(target).value = undefined;
              if (!sameTarget(target, visible)) {
                // Navigated away meanwhile: nothing to report; serve the coalesced fetch if any.
                if (again) continue;
                return;
              }
              // Report once: later refetches fetch the views list instead of the gone list.
              visible = "views";
              for (const cb of goneCbs) cb(target);
              if (again) continue;
              throw e;
            }
            viewSignal(target).value = v;
            if (target.view === "today") noteCount(t, v.open_count);
          }
          // The badge needs Today's count whichever screen is shown; Today and the views list
          // carry it themselves. Not awaited: a failure costs only the badge.
          if (typeof target === "string" ? target !== "views" : isTask(target) || target.view !== "today") {
            fetchList().catch(() => {});
          }
          // Only the visible view's own fetch makes it fresh; another view's does not.
          if (sock.current === "open" && sameTarget(target, visible)) fresh.value = true;
        } while (again);
      } finally {
        fetching = undefined;
      }
    })();
    return fetching;
  }
  const refetchQuietly = () => {
    fetchVisible().catch(() => {});
  };

  sock.onState((s, reason) => {
    if (unpaired) return;
    connection.value = s;
    if (s !== "open") fresh.value = false;
    if (s === "open") {
      sock
        .call("events.subscribe", {})
        .then(fetchVisible)
        .catch(() => {});
    } else if (s === "closed" && reason !== "closed by the page") {
      void checkSession();
    }
  });

  sock.onNotification((method, params) => {
    if (method === "missed") {
      refetchQuietly();
    } else if (method === "event") {
      const t = (params as { type?: string } | undefined)?.type;
      if (t === "data.changed" || t === "status.changed") refetchQuietly();
      else if (t === "op.finished") {
        const e = params as { request_id?: string | null; state?: { state?: string; message?: string } };
        const p = e.request_id ? pending.get(e.request_id) : undefined;
        if (e.request_id) pending.delete(e.request_id);
        const st = e.state?.state;
        if (p && (st === "refused" || st === "cancelled")) {
          const message = e.state?.message ?? "The change was not applied";
          for (const cb of refusalCbs) cb({ ...p, message } as Refusal);
        }
      }
    }
  });

  const isVisible = () => document.visibilityState === "visible";
  const onVisibility = () => {
    if (unpaired) return;
    if (!isVisible()) {
      fresh.value = false;
      return;
    }
    fresh.value = false;
    sock.reconnect(); // a no-op unless closed; the open handler then fetches
    // On an open socket this doubles as a liveness check: a timeout reconnects.
    fetchVisible().catch(() => sock.reconnect());
  };
  document.addEventListener("visibilitychange", onVisibility);
  const interval = setInterval(() => {
    if (isVisible() && !unpaired) refetchQuietly();
  }, PERIOD_MS);

  // Without a session the upgrade would be refused; check first.
  void checkSession();

  async function submit(key: string, method: "op.submit" | "op.undo", extra: object, what: Pending, explicit?: BasedOn): Promise<WriteResult> {
    // Keep the id only for a retry of the same action, inside the retry window.
    if (!kept || kept.key !== key || now() - kept.at > RETRY_WINDOW_MS) {
      if (kept) pending.delete(kept.id); // an abandoned id can no longer be retried
      kept = { key, id: newRequestId(), at: now() };
    }
    const mine = kept;
    // The basis is what the user sees now, captured at invocation; a newer fetch never replaces
    // it (the daemon refuses stale writes). With no view displayed yet, nothing was seen, so
    // the first fetch below supplies it.
    // A views list carries no basis; the opened view's does.
    const basisOf = (v: Visible) =>
      typeof v === "string" ? undefined : isTask(v) ? taskSignal(v.task) : viewSignal(v);
    const basis = basisOf(visible);
    const shown = basis?.value;
    let basedOn: BasedOn | undefined = explicit ?? (shown && { epoch: shown.epoch, revision: shown.revision });
    pending.set(mine.id, what);
    // Wait for the connection, then for data no older than this socket.
    await sock.ready(READY_MS);
    if (method === "op.submit" && !basedOn) {
      await fetchVisible();
      const v = basisOf(visible)?.value;
      if (v) basedOn = { epoch: v.epoch, revision: v.revision };
    }
    const params =
      method === "op.submit"
        ? { session: "", request_id: mine.id, based_on: basedOn, ...extra }
        : { session: "", request_id: mine.id };
    try {
      const r = await sock.call<WriteResult>(method, params);
      if (kept === mine) kept = undefined;
      // Only a queued write can be finished later.
      if (r.outcome !== "queued") pending.delete(mine.id);
      return r;
    } catch (e) {
      // A reply (even a refusal) settles the request; a lost connection does not.
      if (e instanceof Error && e.name === "RpcError") {
        if (kept === mine) kept = undefined;
        pending.delete(mine.id);
        if ((e as { data?: { kind?: string } }).data?.kind === "stale") {
          fresh.value = false;
          refetchQuietly();
        }
      }
      throw e;
    }
  }

  return {
    view: (ref) => viewSignal(ref),
    task: (uid) => taskSignal(uid),
    viewsList,
    onTodayCount(cb) {
      countCbs.add(cb);
      return () => countCbs.delete(cb);
    },
    settings,
    connection,
    fresh,
    async loadViewsList() {
      await sock.ready(READY_MS);
      await fetchList();
    },
    refresh(v) {
      if (!sameTarget(visible, v)) fresh.value = false; // unconfirmed until fetched
      visible = v;
      return fetchVisible();
    },
    onGone(cb) {
      goneCbs.add(cb);
      return () => goneCbs.delete(cb);
    },
    onTaskGone(cb) {
      taskGoneCbs.add(cb);
      return () => taskGoneCbs.delete(cb);
    },
    reopen(uid) {
      const op: Op = { Reopen: { uid } };
      return submit(`reopen:${uid}`, "op.submit", { op }, { kind: "reopen", uid });
    },
    complete(uid) {
      const op: Op = { Complete: { uid } };
      return submit(`complete:${uid}`, "op.submit", { op }, { kind: "complete", uid });
    },
    undo() {
      return submit("undo", "op.undo", {}, { kind: "undo" });
    },
    checkForm: (p) => sock.call<TasksFormResult>("tasks.form", p),
    save(op, basedOn, what) {
      const pend = (what.uid ? { kind: "save", ...what } : { kind: "add", ...what }) as Pending;
      return submit(`save:${JSON.stringify(op)}`, "op.submit", { op }, pend, basedOn);
    },
    remove(uid, title) {
      const op: Op = { Delete: { uid } };
      return submit(`delete:${uid}`, "op.submit", { op }, { kind: "delete", uid, title });
    },
    onRefusal(cb) {
      refusalCbs.add(cb);
      return () => refusalCbs.delete(cb);
    },
    dispose() {
      document.removeEventListener("visibilitychange", onVisibility);
      clearInterval(interval);
      sock.close();
    },
  };
}
