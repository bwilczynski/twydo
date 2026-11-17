// Test doubles shared by the data-layer tests: a scriptable WebSocket.
type Handler = (params: any) => unknown;

export class FakeRpcError {
  constructor(
    public code: number,
    public message: string,
    public kind?: string,
  ) {}
}

export class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  /** Auto-replies by method, applied to every instance. */
  static handlers: Record<string, Handler> = {};
  static reset(): void {
    FakeWebSocket.instances = [];
    FakeWebSocket.handlers = {};
  }
  static get last(): FakeWebSocket {
    return FakeWebSocket.instances[FakeWebSocket.instances.length - 1]!;
  }

  readyState = 0;
  sent: { id?: number; method: string; params: any }[] = [];
  onopen: ((e: unknown) => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: ((e: { code: number; reason: string }) => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;

  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    const msg = JSON.parse(data);
    this.sent.push(msg);
    const h = FakeWebSocket.handlers[msg.method];
    if (h && msg.id !== undefined) {
      queueMicrotask(() => {
        try {
          this.reply(msg.id, h(msg.params));
        } catch (e) {
          if (e instanceof FakeRpcError) this.fail(msg.id, e.code, e.message, e.kind);
          else throw e;
        }
      });
    }
  }

  /** The page closes the socket. */
  close(code = 1000, reason = ""): void {
    this.drop(code, reason);
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.({});
  }
  reply(id: number, result: unknown): void {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", id, result }) });
  }
  fail(id: number, code: number, message: string, kind?: string): void {
    const error: any = { code, message };
    if (kind) error.data = { kind };
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", id, error }) });
  }
  notify(method: string, params: unknown): void {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", method, params }) });
  }
  /** The server (or network) closes the socket. */
  drop(code = 1006, reason = ""): void {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
  calls(method: string) {
    return this.sent.filter((m) => m.method === method);
  }
}

export const asWs = FakeWebSocket as unknown as typeof WebSocket;
