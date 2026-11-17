import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RpcSocket } from "./rpc";
import { asWs, FakeWebSocket } from "./testutil";

beforeEach(() => {
  vi.useFakeTimers();
  FakeWebSocket.reset();
});
afterEach(() => vi.useRealTimers());

describe("RpcSocket", () => {
  it("matches replies by id", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    const ws = FakeWebSocket.last;
    ws.open();
    const a = s.call<string>("a", {});
    const b = s.call<string>("b", {});
    const [ma, mb] = ws.sent;
    ws.reply(mb!.id!, "B");
    ws.reply(ma!.id!, "A");
    expect(await a).toBe("A");
    expect(await b).toBe("B");
    expect(ma).toMatchObject({ jsonrpc: "2.0", method: "a", params: {} });
  });

  it("rejects errors with their kind", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    const ws = FakeWebSocket.last;
    ws.open();
    const p = s.call("a", {});
    ws.fail(ws.sent[0]!.id!, -32000, "stale", "stale");
    await expect(p).rejects.toMatchObject({ code: -32000, data: { kind: "stale" } });
  });

  it("rejects calls while closed", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    FakeWebSocket.last.open();
    FakeWebSocket.last.drop(1011, "daemon not running");
    await expect(s.call("a", {})).rejects.toThrow();
  });

  it("fails in-flight calls when the socket closes and reports the reason", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    const states: [string, string | undefined][] = [];
    s.onState((st, r) => states.push([st, r]));
    FakeWebSocket.last.open();
    const p = s.call("a", {});
    FakeWebSocket.last.drop(1008, "device revoked");
    await expect(p).rejects.toThrow();
    expect(states.at(-1)).toEqual(["closed", "device revoked"]);
  });

  it("delivers notifications", () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    const got: unknown[] = [];
    s.onNotification((m, p) => got.push([m, p]));
    FakeWebSocket.last.open();
    FakeWebSocket.last.notify("event", { type: "data.changed", seq: 1 });
    expect(got).toEqual([["event", { type: "data.changed", seq: 1 }]]);
  });

  it("reconnects with backoff", () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.last.drop();
    vi.advanceTimersByTime(999);
    expect(FakeWebSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.last.drop();
    vi.advanceTimersByTime(1999);
    expect(FakeWebSocket.instances).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(FakeWebSocket.instances).toHaveLength(3);
    // a successful open resets the delay
    FakeWebSocket.last.open();
    FakeWebSocket.last.drop();
    vi.advanceTimersByTime(1000);
    expect(FakeWebSocket.instances).toHaveLength(4);
    // the delay caps at 30 s
    for (let i = 0; i < 8; i++) {
      FakeWebSocket.last.drop();
      vi.advanceTimersByTime(30000);
    }
    const n = FakeWebSocket.instances.length;
    FakeWebSocket.last.drop();
    vi.advanceTimersByTime(29999);
    expect(FakeWebSocket.instances).toHaveLength(n);
    vi.advanceTimersByTime(1);
    expect(FakeWebSocket.instances).toHaveLength(n + 1);
    s.close();
  });

  it("does not reconnect after close()", () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    s.close();
    vi.advanceTimersByTime(60000);
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it("reconnects at once on request", () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    FakeWebSocket.last.drop();
    s.reconnect();
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it("times out a call that is never answered and reconnects", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    FakeWebSocket.last.open();
    const p = s.call("a", {});
    p.catch(() => {});
    await vi.advanceTimersByTimeAsync(10000);
    await expect(p).rejects.toThrow(/timed out/);
    expect(FakeWebSocket.instances).toHaveLength(2);
    s.close();
  });

  it("replaces an open socket that goes silent", async () => {
    const s = new RpcSocket("ws://x/ws", asWs);
    FakeWebSocket.last.open();
    await vi.advanceTimersByTimeAsync(30000);
    FakeWebSocket.last.notify("heartbeat", { seq: 1 });
    await vi.advanceTimersByTimeAsync(30000);
    expect(FakeWebSocket.instances).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(15000);
    expect(FakeWebSocket.instances).toHaveLength(2);
    s.close();
  });
});
