// The only module screens use to reach the daemon (design §9.4).
import { signal, type ReadonlySignal } from "@preact/signals";
import type {
  BasedOn,
  CaptureTarget,
  NewTask,
  Op,
  ViewsGetResult,
  WriteResult,
} from "./gen/protocol";
import { RpcSocket } from "./rpc";

export type ViewName = "inbox" | "today";
export type Connection = "connecting" | "open" | "closed" | "unpaired";

export interface DataSource {
  view(name: ViewName): ReadonlySignal<ViewsGetResult | undefined>;
  connection: ReadonlySignal<Connection>;
  /** True only after a successful fetch on the current open socket; false when hidden or closed. */
  fresh: ReadonlySignal<boolean>;
  refresh(name: ViewName): Promise<void>;
  /** based_on comes from the last view fetched. */
  complete(uid: string): Promise<WriteResult>;
  create(title: string, note: string, capture: CaptureTarget): Promise<WriteResult>;
  undo(): Promise<WriteResult>;
  /** Stop listening and close the socket. */
  dispose(): void;
}

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

  const views: Record<ViewName, ReturnType<typeof signal<ViewsGetResult | undefined>>> = {
    inbox: signal(undefined),
    today: signal(undefined),
  };
  const connection = signal<Connection>("connecting");
  let visible: ViewName = "inbox";
  let basedOn: BasedOn | undefined;
  let unpaired = false;
  const fresh = signal(false);
  const now = opts.now ?? Date.now;
  // The one action that lost its connection before a reply; a retry of it reuses the id.
  let kept: { key: string; id: string; at: number } | undefined;

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
          const name = visible;
          const v = await sock.call<ViewsGetResult>("views.get", { view: name });
          views[name].value = v;
          basedOn = { epoch: v.epoch, revision: v.revision };
          if (sock.current === "open") fresh.value = true;
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

  async function submit(key: string, method: "op.submit" | "op.undo", extra: object): Promise<WriteResult> {
    // Keep the id only for a retry of the same action, inside the retry window.
    if (!kept || kept.key !== key || now() - kept.at > RETRY_WINDOW_MS) {
      kept = { key, id: newRequestId(), at: now() };
    }
    const mine = kept;
    // Wait for the connection, then for data no older than this socket.
    await sock.ready(READY_MS);
    if (method === "op.submit" && (!basedOn || !fresh.value)) await fetchVisible();
    const params =
      method === "op.submit"
        ? { session: "", request_id: mine.id, based_on: basedOn, ...extra }
        : { session: "", request_id: mine.id };
    try {
      const r = await sock.call<WriteResult>(method, params);
      if (kept === mine) kept = undefined;
      return r;
    } catch (e) {
      // A reply (even a refusal) settles the request; a lost connection does not.
      if (e instanceof Error && e.name === "RpcError") {
        if (kept === mine) kept = undefined;
        if ((e as { data?: { kind?: string } }).data?.kind === "stale") {
          fresh.value = false;
          refetchQuietly();
        }
      }
      throw e;
    }
  }

  return {
    view: (name) => views[name],
    connection,
    fresh,
    refresh(name) {
      visible = name;
      return fetchVisible();
    },
    complete(uid) {
      const op: Op = { Complete: { uid } };
      return submit(`complete:${uid}`, "op.submit", { op });
    },
    create(title, note, capture) {
      const task: NewTask = {
        collection: capture.collection,
        summary: title,
        description: note === "" ? null : note,
        kind: "Task",
        due: capture.due ? { Date: capture.due } : null,
      };
      const op: Op = { Create: task };
      return submit(`create:${capture.collection}\0${capture.due ?? ""}\0${title}\0${note}`, "op.submit", { op });
    },
    undo() {
      return submit("undo", "op.undo", {});
    },
    dispose() {
      document.removeEventListener("visibilitychange", onVisibility);
      clearInterval(interval);
      sock.close();
    },
  };
}
