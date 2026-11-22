import { signal, type ReadonlySignal } from "@preact/signals";

export type PushState =
  "unsupported" | "install" | "denied" | "off" | "on" | "busy";

export interface PushEnv {
  fetch: typeof fetch;
  registration: Promise<ServiceWorkerRegistration> | undefined;
  notification:
    | {
        permission: NotificationPermission;
        requestPermission(): Promise<NotificationPermission>;
      }
    | undefined;
  standalone: boolean;
  /** Remembers that the user turned reminders off; a restart must not resubscribe. */
  storage?: Pick<Storage, "getItem" | "setItem" | "removeItem">;
}

export interface Push {
  state: ReadonlySignal<PushState>;
  /** Turning reminders off failed on the server; cleared by the next enable or disable. */
  error: ReadonlySignal<boolean>;
  enable(): Promise<void>;
  disable(): Promise<void>;
  resync(): Promise<void>;
}

/** The page's localStorage, or undefined where reading it throws (blocked site data). */
export function browserStorage(): PushEnv["storage"] {
  try {
    const s = window.localStorage;
    s.getItem(OFF_KEY);
    return s;
  } catch {
    return undefined;
  }
}

const OFF_KEY = "twydo.reminders.off";
const SAME = { credentials: "same-origin" } as const;

function decodeKey(key: string): Uint8Array {
  const b64 = key
    .replace(/-/g, "+")
    .replace(/_/g, "/")
    .padEnd(Math.ceil(key.length / 4) * 4, "=");
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

/**
 * The phone's Web Push subscription (spec §8.2). The browser's own subscription is the switch:
 * none means off, so nothing is stored here.
 */
export function createPush(env: PushEnv): Push {
  const { registration, notification } = env;
  const state = signal<PushState>("busy");
  const error = signal(false);
  const subscription = async () =>
    (await registration!).pushManager.getSubscription();

  const wantedOff = () => {
    try {
      return env.storage?.getItem(OFF_KEY) === "1";
    } catch {
      return false;
    }
  };
  const remember = (off: boolean) => {
    try {
      if (off) env.storage?.setItem(OFF_KEY, "1");
      else env.storage?.removeItem(OFF_KEY);
    } catch {
      // storage unavailable: behave as without a preference
    }
  };

  const put = (sub: PushSubscription) =>
    env.fetch("/push/subscription", {
      ...SAME,
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(sub.toJSON()),
    });

  if (!registration || !notification) state.value = "unsupported";
  else if (!env.standalone) state.value = "install";
  else if (notification.permission === "denied") state.value = "denied";
  else
    subscription().then(
      (s) => {
        if (wantedOff()) {
          // A subscription the user turned off may linger in the browser: drop it again.
          void s?.unsubscribe().catch(() => {});
          if (state.value === "busy") state.value = "off";
        } else if (state.value === "busy") state.value = s ? "on" : "off";
      },
      () => {
        state.value = "unsupported";
      },
    );

  return {
    state,
    error,

    async enable() {
      if (!registration || !notification) return;
      state.value = "busy";
      error.value = false;
      try {
        const answer = await notification.requestPermission();
        if (answer !== "granted") {
          state.value = answer === "denied" ? "denied" : "off";
          return;
        }
        const res = await env.fetch("/push/key", SAME);
        if (res.status === 404) {
          state.value = "unsupported";
          return;
        }
        if (!res.ok) throw new Error(`push key: ${res.status}`);
        const { key } = (await res.json()) as { key: string };
        const sub = await (
          await registration
        ).pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: decodeKey(key) as BufferSource,
        });
        // A subscription the server does not hold must not stay in the browser.
        const drop = () => sub.unsubscribe().catch(() => {});
        const saved = await put(sub).catch(() => undefined);
        if (!saved?.ok) {
          await drop();
          state.value = "off";
          return;
        }
        remember(false);
        state.value = "on";
      } catch {
        state.value = "off";
      }
    },

    async disable() {
      if (!registration) return;
      state.value = "busy";
      error.value = false;
      try {
        await (await subscription())?.unsubscribe();
      } catch {
        // the server delete below is what turns reminders off
      }
      const res = await env
        .fetch("/push/subscription", { ...SAME, method: "DELETE" })
        .catch(() => undefined);
      if (res?.ok) {
        remember(true);
        state.value = "off";
      } else {
        // The server may still push here: the switch stays on so it can be tapped again.
        error.value = true;
        state.value = "on";
      }
    },

    async resync() {
      if (!registration || notification?.permission !== "granted") return;
      if (wantedOff()) return;
      try {
        const sub = await subscription();
        if (sub) await put(sub);
      } catch {
        // offline: the next start tries again
      }
    },
  };
}
