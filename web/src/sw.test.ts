import swSource from "../public/sw.js?raw";
import { describe, expect, it, vi } from "vitest";

type Handler = (e: Record<string, unknown>) => void;

function load() {
  const handlers: Record<string, Handler> = {};
  const showNotification = vi.fn(() => Promise.resolve());
  const openWindow = vi.fn(() => Promise.resolve());
  const matchAll = vi.fn(() => Promise.resolve([] as unknown[]));
  const self = {
    addEventListener: (t: string, h: Handler) => (handlers[t] = h),
    registration: { showNotification },
    clients: { matchAll, openWindow, claim: () => Promise.resolve() },
    skipWaiting: () => {},
  };
  const src = swSource;
  new Function("self", "caches", "location", src)(
    self,
    {},
    { origin: "http://x" },
  );
  return { handlers, showNotification, openWindow, matchAll };
}

async function dispatch(h: Handler, extra: Record<string, unknown>) {
  const waits: Promise<unknown>[] = [];
  h({ ...extra, waitUntil: (p: Promise<unknown>) => waits.push(p) });
  await Promise.all(waits);
}

const click = (uid?: string) => {
  const close = vi.fn();
  return { close, e: { notification: { close, data: { uid } } } };
};

describe("service worker push", () => {
  it("push_shows_title_body_and_tag", async () => {
    const t = load();
    const data = {
      json: () => ({ title: "Call", body: "10:00", key: "k1", uid: "u1" }),
    };
    await dispatch(t.handlers.push, { data });
    expect(t.showNotification).toHaveBeenCalledWith("Call", {
      body: "10:00",
      tag: "k1",
      renotify: false,
      data: { uid: "u1" },
    });
  });

  it("push_with_bad_payload_still_shows_a_notification", async () => {
    const t = load();
    const data = {
      json: () => {
        throw new Error("bad");
      },
    };
    await dispatch(t.handlers.push, { data });
    expect(t.showNotification).toHaveBeenCalledWith("Twydo reminder", {});
  });

  it("push_with_odd_payloads_still_shows_a_notification", async () => {
    for (const [payload, expected] of [
      [null, ["Twydo reminder", {}]],
      [{}, ["Twydo reminder", {}]],
      [{ title: 5, body: 7, key: 3, uid: 9 }, ["Twydo reminder", {}]],
      [{ title: "", key: "", uid: "" }, ["Twydo reminder", {}]],
    ] as const) {
      const t = load();
      await dispatch(t.handlers.push, { data: { json: () => payload } });
      expect(t.showNotification).toHaveBeenCalledTimes(1);
      const [title, opts] = t.showNotification.mock.calls[0] as unknown as [
        string,
        Record<string, unknown>,
      ];
      expect(title).toBe(expected[0]);
      expect(opts.body).toBeUndefined();
      expect(opts.tag).toBeUndefined();
      expect((opts.data as { uid?: string }).uid).toBeUndefined();
    }
  });

  it("click_still_posts_open_when_focus_rejects", async () => {
    const t = load();
    const client = {
      type: "window",
      focus: vi.fn(() => Promise.reject(new Error("no"))),
      postMessage: vi.fn(),
    };
    t.matchAll.mockResolvedValue([client]);
    await dispatch(t.handlers.notificationclick, click("u1").e);
    expect(client.postMessage).toHaveBeenCalledWith({
      type: "open",
      uid: "u1",
    });
  });

  it("click_with_open_window_posts_open", async () => {
    const t = load();
    const client = {
      type: "window",
      focus: vi.fn(() => Promise.resolve()),
      postMessage: vi.fn(),
    };
    t.matchAll.mockResolvedValue([client]);
    const c = click("u 1");
    await dispatch(t.handlers.notificationclick, c.e);
    expect(c.close).toHaveBeenCalled();
    expect(client.focus).toHaveBeenCalled();
    expect(client.postMessage).toHaveBeenCalledWith({
      type: "open",
      uid: "u 1",
    });
    expect(t.openWindow).not.toHaveBeenCalled();
  });

  it("click_without_window_opens_task_url", async () => {
    const t = load();
    await dispatch(t.handlers.notificationclick, click("u 1").e);
    expect(t.openWindow).toHaveBeenCalledWith("/#/task/u%201");
  });

  it("digest_click_opens_today", async () => {
    const t = load();
    await dispatch(t.handlers.notificationclick, click().e);
    expect(t.openWindow).toHaveBeenCalledWith("/#/today");
    const client = {
      type: "window",
      focus: vi.fn(() => Promise.resolve()),
      postMessage: vi.fn(),
    };
    t.matchAll.mockResolvedValue([client]);
    await dispatch(t.handlers.notificationclick, click().e);
    expect(client.postMessage).toHaveBeenCalledWith({ type: "open" });
  });
});
