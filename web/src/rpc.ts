// JSON-RPC 2.0 over one WebSocket (one message per text frame), with reconnect.

export type SocketState = "connecting" | "open" | "closed";

export class RpcError extends Error {
  constructor(
    public code: number,
    message: string,
    public data?: { kind?: string; [k: string]: unknown },
  ) {
    super(message);
    this.name = "RpcError";
  }
}

interface Pending {
  resolve: (v: any) => void;
  reject: (e: Error) => void;
}

const MIN_DELAY = 1000;
const MAX_DELAY = 30000;
export const CALL_TIMEOUT = 10000;
// The daemon heartbeats every 15 s; 2.5 beats of silence means the socket is dead.
export const SILENCE_LIMIT = 40000;
const WATCH_EVERY = 5000;

export class RpcSocket {
  private ws: WebSocket | undefined;
  private state: SocketState = "closed";
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private notifyCbs: ((method: string, params: unknown) => void)[] = [];
  private stateCbs: ((s: SocketState, reason?: string) => void)[] = [];
  private delay = MIN_DELAY;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private stopped = false;
  private lastFrame = Date.now();
  private watchdog: ReturnType<typeof setInterval> | undefined;

  constructor(
    private url: string,
    private WS: typeof WebSocket = WebSocket,
  ) {
    this.connect();
    this.watchdog = setInterval(() => {
      if (this.state === "open" && Date.now() - this.lastFrame > SILENCE_LIMIT) {
        this.forceReconnect("no heartbeat");
      }
    }, WATCH_EVERY);
  }

  get current(): SocketState {
    return this.state;
  }

  call<R>(method: string, params: unknown, timeoutMs = CALL_TIMEOUT): Promise<R> {
    if (this.state !== "open" || !this.ws) {
      return Promise.reject(new Error(`not connected (${this.state})`));
    }
    const id = this.nextId++;
    return new Promise<R>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this.pending.delete(id)) return;
        reject(new Error(`${method} timed out`));
        this.forceReconnect("call timed out");
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
      try {
        this.ws!.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      } catch (e) {
        this.pending.delete(id);
        clearTimeout(timer);
        reject(e instanceof Error ? e : new Error(String(e)));
      }
    });
  }

  onNotification(cb: (method: string, params: unknown) => void): void {
    this.notifyCbs.push(cb);
  }

  onState(cb: (s: SocketState, reason?: string) => void): void {
    this.stateCbs.push(cb);
  }

  /** Connect now if the socket is closed (skips the backoff wait). */
  reconnect(): void {
    if (this.stopped || this.state !== "closed") return;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.connect();
  }

  /** Resolve once open, reconnecting if closed; reject after `ms`. */
  ready(ms = 5000): Promise<void> {
    if (this.state === "open") return Promise.resolve();
    if (this.stopped) return Promise.reject(new Error("socket closed"));
    this.reconnect();
    return new Promise((resolve, reject) => {
      const cb = (s: SocketState) => {
        if (s !== "open") return;
        clearTimeout(t);
        this.stateCbs = this.stateCbs.filter((c) => c !== cb);
        resolve();
      };
      const t = setTimeout(() => {
        this.stateCbs = this.stateCbs.filter((c) => c !== cb);
        reject(new Error("not connected"));
      }, ms);
      this.stateCbs.push(cb);
    });
  }

  /** Drop a socket that may be dead without telling us, and connect again at once. */
  private forceReconnect(reason: string): void {
    if (this.stopped) return;
    const ws = this.ws;
    this.ws = undefined;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
    clearTimeout(this.timer);
    this.timer = undefined;
    this.failPending(reason);
    this.setState("closed", reason);
    this.connect();
  }

  close(): void {
    this.stopped = true;
    clearInterval(this.watchdog);
    clearTimeout(this.timer);
    this.timer = undefined;
    const ws = this.ws;
    this.ws = undefined;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
    this.failPending("socket closed");
    if (this.state !== "closed") this.setState("closed", "closed by the page");
  }

  private setState(s: SocketState, reason?: string): void {
    this.state = s;
    for (const cb of this.stateCbs) cb(s, reason);
  }

  private connect(): void {
    this.setState("connecting");
    this.lastFrame = Date.now();
    const ws = new this.WS(this.url);
    this.ws = ws;
    ws.onopen = () => {
      this.delay = MIN_DELAY;
      this.lastFrame = Date.now();
      this.setState("open");
    };
    ws.onmessage = (e) => this.onMessage(String(e.data));
    ws.onclose = (e) => {
      if (this.ws !== ws) return;
      this.ws = undefined;
      this.failPending("connection lost");
      this.setState("closed", e.reason || undefined);
      if (!this.stopped) {
        this.timer = setTimeout(() => {
          this.timer = undefined;
          this.connect();
        }, this.delay);
        this.delay = Math.min(this.delay * 2, MAX_DELAY);
      }
    };
  }

  private failPending(why: string): void {
    const all = [...this.pending.values()];
    this.pending.clear();
    for (const p of all) p.reject(new Error(why));
  }

  private onMessage(text: string): void {
    this.lastFrame = Date.now();
    let msg: any;
    try {
      msg = JSON.parse(text);
    } catch {
      return;
    }
    if (msg && typeof msg.id === "number" && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id)!;
      this.pending.delete(msg.id);
      if (msg.error) {
        p.reject(new RpcError(msg.error.code, msg.error.message, msg.error.data));
      } else {
        p.resolve(msg.result);
      }
    } else if (msg && typeof msg.method === "string") {
      for (const cb of this.notifyCbs) cb(msg.method, msg.params);
    }
  }
}
