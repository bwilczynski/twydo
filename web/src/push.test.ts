import { afterEach, describe, expect, it, vi } from "vitest";
import { browserStorage, createPush, type PushEnv } from "./push";

const json = {
  endpoint: "https://push.example/abc",
  keys: { p256dh: "pk", auth: "ak" },
};

function setup(over: Partial<PushEnv> = {}, existing = false, keyStatus = 200) {
  const unsubscribe = vi.fn(async () => true);
  let sub: unknown = existing ? { toJSON: () => json, unsubscribe } : null;
  const subscribe = vi.fn(async (_o: unknown) => {
    sub = { toJSON: () => json, unsubscribe };
    return sub;
  });
  const registration = Promise.resolve({
    pushManager: { getSubscription: async () => sub, subscribe },
  } as unknown as ServiceWorkerRegistration);
  const calls: { url: string; init?: RequestInit }[] = [];
  const fetchStub = vi.fn(async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    if (url === "/push/key") {
      return keyStatus === 200
        ? new Response(JSON.stringify({ key: "AQID" }), { status: 200 })
        : new Response("", { status: keyStatus });
    }
    return new Response(null, { status: 204 });
  });
  const notification = {
    permission: "default" as NotificationPermission,
    requestPermission: vi.fn(async () => "granted" as NotificationPermission),
  };
  const env: PushEnv = {
    fetch: fetchStub as unknown as typeof fetch,
    registration,
    notification,
    standalone: true,
    ...over,
  };
  return { env, calls, subscribe, unsubscribe, notification };
}

const settle = (push: ReturnType<typeof createPush>) =>
  vi.waitFor(() => expect(push.state.value).not.toBe("busy"));

describe("createPush", () => {
  it("state_unsupported_without_service_worker", () => {
    const { env } = setup({ registration: undefined });
    expect(createPush(env).state.value).toBe("unsupported");
  });

  it("state_install_in_a_browser_tab", () => {
    const { env } = setup({ standalone: false });
    expect(createPush(env).state.value).toBe("install");
  });

  it("state_denied_and_on", async () => {
    const a = setup();
    a.notification.permission = "denied";
    expect(createPush(a.env).state.value).toBe("denied");
    const b = setup({}, true);
    const push = createPush(b.env);
    await settle(push);
    expect(push.state.value).toBe("on");
  });

  it("enable_subscribes_and_puts", async () => {
    const { env, calls, subscribe } = setup();
    const push = createPush(env);
    await settle(push);
    expect(push.state.value).toBe("off");
    await push.enable();
    const arg = subscribe.mock.calls[0]![0] as {
      userVisibleOnly: boolean;
      applicationServerKey: Uint8Array;
    };
    expect(arg.userVisibleOnly).toBe(true);
    expect([...arg.applicationServerKey]).toEqual([1, 2, 3]);
    const put = calls.find((c) => c.init?.method === "PUT")!;
    expect(put.url).toBe("/push/subscription");
    expect(JSON.parse(put.init!.body as string)).toEqual(json);
    expect(push.state.value).toBe("on");
  });

  it("enable_unsubscribes_when_the_put_throws", async () => {
    const { env, unsubscribe } = setup();
    const inner = env.fetch;
    env.fetch = (async (url: string, init?: RequestInit) => {
      if (init?.method === "PUT") throw new TypeError("offline");
      return inner(url, init);
    }) as unknown as typeof fetch;
    const push = createPush(env);
    await settle(push);
    await push.enable();
    expect(unsubscribe).toHaveBeenCalled();
    expect(push.state.value).toBe("off");
  });

  it("enable_denied_stays_denied", async () => {
    const { env, calls, subscribe, notification } = setup();
    notification.requestPermission.mockResolvedValue("denied");
    const push = createPush(env);
    await settle(push);
    await push.enable();
    expect(push.state.value).toBe("denied");
    expect(subscribe).not.toHaveBeenCalled();
    expect(calls).toEqual([]);
  });

  it("enable_on_insecure_gateway_is_unsupported", async () => {
    const { env, subscribe } = setup({}, false, 404);
    const push = createPush(env);
    await settle(push);
    await push.enable();
    expect(push.state.value).toBe("unsupported");
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("disable_unsubscribes_and_deletes", async () => {
    const { env, calls, unsubscribe } = setup({}, true);
    const push = createPush(env);
    await settle(push);
    await push.disable();
    expect(unsubscribe).toHaveBeenCalled();
    expect(
      calls.some(
        (c) => c.url === "/push/subscription" && c.init?.method === "DELETE",
      ),
    ).toBe(true);
    expect(push.state.value).toBe("off");
  });

  describe("disable failures", () => {
    const deletes = (calls: { url: string; init?: RequestInit }[]) =>
      calls.filter((c) => c.url === "/push/subscription" && c.init?.method === "DELETE");

    it("disable_deletes_when_unsubscribe_rejects", async () => {
      const { env, calls, unsubscribe } = setup({}, true);
      unsubscribe.mockRejectedValue(new Error("boom"));
      const push = createPush(env);
      await settle(push);
      await push.disable();
      expect(deletes(calls)).toHaveLength(1);
      expect(push.state.value).toBe("off");
      expect(push.error.value).toBe(false);
    });

    it("disable_deletes_when_unsubscribe_returns_false", async () => {
      const { env, calls, unsubscribe } = setup({}, true);
      unsubscribe.mockResolvedValue(false);
      const push = createPush(env);
      await settle(push);
      await push.disable();
      expect(deletes(calls)).toHaveLength(1);
      expect(push.state.value).toBe("off");
    });

    it("disable_deletes_when_get_subscription_rejects", async () => {
      const { env, calls } = setup({
        registration: Promise.resolve({
          pushManager: { getSubscription: async () => Promise.reject(new Error("boom")) },
        } as unknown as ServiceWorkerRegistration),
      });
      const push = createPush(env);
      await vi.waitFor(() => expect(push.state.value).toBe("unsupported"));
      await push.disable();
      expect(deletes(calls)).toHaveLength(1);
      expect(push.state.value).toBe("off");
    });

    it("disable_server_500_is_not_off_and_sets_error", async () => {
      const { env } = setup({}, true);
      const inner = env.fetch;
      env.fetch = (async (url: string, init?: RequestInit) =>
        init?.method === "DELETE" ? new Response("", { status: 500 }) : inner(url, init)) as unknown as typeof fetch;
      const push = createPush(env);
      await settle(push);
      await push.disable();
      expect(push.state.value).toBe("on");
      expect(push.error.value).toBe(true);
    });

    it("disable_delete_throwing_is_not_off_and_sets_error_then_clears", async () => {
      const { env } = setup({}, true);
      const inner = env.fetch;
      let fail = true;
      env.fetch = (async (url: string, init?: RequestInit) => {
        if (init?.method === "DELETE" && fail) throw new TypeError("offline");
        return inner(url, init);
      }) as unknown as typeof fetch;
      const push = createPush(env);
      await settle(push);
      await push.disable();
      expect(push.state.value).not.toBe("off");
      expect(push.error.value).toBe(true);
      fail = false;
      await push.disable();
      expect(push.state.value).toBe("off");
      expect(push.error.value).toBe(false);
    });
  });

  describe("off preference", () => {
    const memory = () => {
      const m = new Map<string, string>();
      return {
        getItem: (k: string) => m.get(k) ?? null,
        setItem: (k: string, v: string) => void m.set(k, v),
        removeItem: (k: string) => void m.delete(k),
      };
    };
    const puts = (calls: { url: string; init?: RequestInit }[]) =>
      calls.filter((c) => c.init?.method === "PUT");

    for (const how of ["rejects", "false"] as const) {
      it(`restart_after_off_stays_off_when_unsubscribe_${how}`, async () => {
        const storage = memory();
        const a = setup({ storage }, true);
        if (how === "rejects") a.unsubscribe.mockRejectedValue(new Error("x"));
        else a.unsubscribe.mockResolvedValue(false);
        a.notification.permission = "granted";
        const first = createPush(a.env);
        await settle(first);
        await first.disable();
        expect(first.state.value).toBe("off");
        const b = setup({ storage }, true);
        b.notification.permission = "granted";
        const second = createPush(b.env);
        await settle(second);
        expect(second.state.value).toBe("off");
        await second.resync();
        expect(puts(b.calls)).toEqual([]);
        expect(b.unsubscribe).toHaveBeenCalled();
      });
    }

    it("enable_clears_the_preference", async () => {
      const storage = memory();
      storage.setItem("twydo.reminders.off", "1");
      const a = setup({ storage });
      const push = createPush(a.env);
      await settle(push);
      await push.enable();
      expect(storage.getItem("twydo.reminders.off")).toBeNull();
      const b = setup({ storage }, true);
      b.notification.permission = "granted";
      const next = createPush(b.env);
      await settle(next);
      expect(next.state.value).toBe("on");
      await next.resync();
      expect(puts(b.calls)).toHaveLength(1);
    });

    it("throwing_storage_does_not_crash", async () => {
      const boom = () => {
        throw new Error("denied");
      };
      const storage = { getItem: boom, setItem: boom, removeItem: boom };
      const a = setup({ storage }, true);
      const push = createPush(a.env);
      await settle(push);
      expect(push.state.value).toBe("on");
      await push.disable();
      expect(push.state.value).toBe("off");
      await push.enable();
      expect(push.state.value).toBe("on");
    });
  });

  describe("browserStorage", () => {
    afterEach(() => {
      vi.restoreAllMocks();
      localStorage.clear();
    });

    it("returns_the_local_storage", () => {
      localStorage.clear();
      browserStorage()!.setItem("k", "v");
      expect(localStorage.getItem("k")).toBe("v");
    });

    it("returns_undefined_when_the_getter_throws", () => {
      vi.spyOn(window, "localStorage", "get").mockImplementation(() => {
        throw new Error("denied");
      });
      expect(browserStorage()).toBeUndefined();
    });

    it("off_choice_survives_a_restart_with_the_real_wiring", async () => {
      localStorage.clear();
      const a = setup({ storage: browserStorage() }, true);
      a.unsubscribe.mockRejectedValue(new Error("x"));
      a.notification.permission = "granted";
      const first = createPush(a.env);
      await settle(first);
      await first.disable();
      expect(first.state.value).toBe("off");
      const b = setup({ storage: browserStorage() }, true);
      b.notification.permission = "granted";
      const second = createPush(b.env);
      await settle(second);
      expect(second.state.value).toBe("off");
      await second.resync();
      expect(b.calls.filter((c) => c.init?.method === "PUT")).toEqual([]);
    });
  });

  it("resync_puts_an_existing_subscription", async () => {
    const { env, calls, notification } = setup({}, true);
    notification.permission = "granted";
    const push = createPush(env);
    await push.resync();
    const put = calls.find((c) => c.init?.method === "PUT")!;
    expect(JSON.parse(put.init!.body as string)).toEqual(json);
  });

  it("resync_without_permission_does_nothing", async () => {
    const { env, calls } = setup({}, true);
    await createPush(env).resync();
    expect(calls).toEqual([]);
  });
});
