// Cache-first for built assets, network-first for pages. Never touches the socket or auth routes.
const CACHE = "twydo-v1";
const NEVER = ["/ws", "/pair", "/session"];

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches
      .keys()
      .then((ks) =>
        Promise.all(ks.filter((k) => k !== CACHE).map((k) => caches.delete(k))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== location.origin || NEVER.includes(url.pathname)) return;
  if (url.pathname.startsWith("/assets/")) {
    e.respondWith(
      caches.match(req).then(
        (hit) =>
          hit ||
          fetch(req).then((res) => {
            if (res.ok) {
              const copy = res.clone();
              caches.open(CACHE).then((c) => c.put(req, copy));
            }
            return res;
          }),
      ),
    );
  } else if (req.mode === "navigate") {
    e.respondWith(
      fetch(req)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put("/", copy));
          }
          return res;
        })
        .catch(() => caches.match("/").then((hit) => hit || Response.error())),
    );
  }
});

// The icon badge: Today's open count; 0 removes it. Never rejects, never throws: a badge
// failure must not cost the notification.
function badge(n) {
  if (!Number.isInteger(n) || n < 0) return Promise.resolve();
  try {
    const nav = self.navigator;
    const p =
      n === 0
        ? nav && nav.clearAppBadge && nav.clearAppBadge()
        : nav && nav.setAppBadge && nav.setAppBadge(n);
    return Promise.resolve(p).catch(() => {});
  } catch {
    return Promise.resolve();
  }
}

// Web Push: show the reminder; a tap focuses the app on its task (or Today for the digest).
self.addEventListener("push", (e) => {
  let p;
  try {
    p = e.data.json();
  } catch {
    e.waitUntil(self.registration.showNotification("Twydo reminder", {}));
    return;
  }
  // Every push must show a notification (iOS revokes push otherwise), whatever the payload.
  const o = p && typeof p === "object" ? p : {};
  const text = (v) => (typeof v === "string" && v ? v : undefined);
  const opts = { renotify: false, data: {} };
  if (typeof o.body === "string") opts.body = o.body;
  if (text(o.key)) opts.tag = o.key;
  if (text(o.uid)) opts.data.uid = o.uid;
  e.waitUntil(
    Promise.all([
      badge(o.badge),
      self.registration.showNotification(text(o.title) || "Twydo reminder", opts),
    ]),
  );
});

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const uid = e.notification.data && e.notification.data.uid;
  const target = uid ? "/#/task/" + encodeURIComponent(uid) : "/#/today";
  e.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((all) => {
        const client = all.find((c) => c.type === "window");
        if (!client) return self.clients.openWindow(target);
        // A rejected focus (the browser may refuse) must not lose the navigation.
        return Promise.resolve()
          .then(() => client.focus())
          .catch(() => {})
          .then(() =>
            client.postMessage(uid ? { type: "open", uid } : { type: "open" }),
          );
      }),
  );
});
