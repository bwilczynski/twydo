# Web Push reminders on the phone

Date: 2026-11-22. Status: draft for review. Follows the
[phone companion MVP](2026-11-17-phone-companion-mvp-design.md) §10 item 2 and the
[notifier design](2026-11-14-notifier-design.md) ("Later sinks … Web Push plug in here").
The [main spec](2026-10-31-terminal-tasks-design.md) §7 notification delivery contract, as the
notifier design amends it, applies unchanged.

Points marked *(my call)* are proposals the builder may refine with a reason. Everything else is
the user's decision or follows from the existing code.

## 1. Intent

The user wants Twydo to **replace 2Do's reminders on the phone** (user, 2026-11-22). Once this
works, 2Do's notifications on the iPhone are turned off, so the phone's only reminders are the
ones Twydo pushes. A missed reminder on the phone is therefore the failure the notifier design
already treats as an anti-goal; a duplicate is still an accepted cost.

The phone gets the same reminders the desktop gets, from the same planner: alerts, timed dues
and the morning digest, with the same catch-up rule.

## 2. Goals, non-goals, anti-goals

**Goals**

- Every reminder the server's notifier plans reaches each phone that has turned reminders on,
  as a system notification, with the home-screen app closed.
- A failed push is retried by the notifier, exactly like a failed desktop notification.
- A reminder that cannot be pushed is visible **on the phone**, not only in `tdo status` on a
  server nobody looks at.
- Tapping a task's reminder opens that task in the app; tapping the digest opens Today.
- Turning reminders on and off from the phone, per device.

**Non-goals**

- Complete or Snooze actions on the notification (notifier design: plain reminders).
- Per-device or per-list choice of which reminders to receive.
- Push to the desktop browser, or to a `display` device.
- Suppressing the desktop popup when the phone was notified, or the reverse. The laptop's own
  daemon keeps notifying; the same reminder on both is the accepted duplicate.
- Push in `--insecure` (plain HTTP) mode: browsers only offer push in a secure context.

**Anti-goals** (failure even if it works)

- A reminder recorded as sent although no phone accepted it.
- Task titles or notes readable by the push service (Apple, Google, Mozilla) or in any log.
- The daemon gaining network, HTTP-client or Web Push code.
- A page on another origin able to change a device's push subscription.
- A revoked device still receiving reminders.

## 3. Approaches

| | Approach | Trade-off | Outcome |
|---|---|---|---|
| A | **The notifier's command sink runs `tdo gateway push`**, which reads the gateway's subscriptions and sends the push. | Reuses retry, catch-up, "sent only on success" and `notifier_error` as they are; one short process per reminder; one small daemon change (pass the reminder's key). | **Chosen** (user, 2026-11-22). |
| B | A Web Push sink inside the daemon, subscriptions held by the daemon. | No extra process; puts network and crypto code into the process that owns the data, which the MVP design rejected. | Rejected. |
| C | The daemon emits a `reminder.due` event; the running gateway pushes. | Needs a new acknowledgement path back to the notifier, or reminders falling due while the gateway restarts are lost. | Rejected: more work for the same guarantee as A. |

## 4. Architecture

```
notifier thread ──CommandSink──▶ tdo gateway push ──HTTPS (RFC 8030/8291/8292)──▶ push service ──▶ phone SW
 (daemon, server)   title, body       reads gateway state:                         (Apple)          showNotification
                    + env key/uid     VAPID key, subscriptions
                         ▲
                         └── exit 0 only when every subscription accepted (or is gone)
```

On the server, `config.toml` gets:

```toml
[notifications]
command = ["tdo", "gateway", "push"]
```

This works with the image's `TWYDO_NOTIFIER=off`, which only removes the platform sink; a
configured command is used regardless (as `docs/config.md` already says). The image needs no
change to its environment; `docs/gateway.md` documents the key.

## 5. Daemon change: the reminder's key for the command

`CommandSink` sets two environment variables on the command it starts *(my call: environment,
not extra arguments, so existing user commands that take the last two arguments as title and
body keep working)*:

- `TWYDO_REMINDER_KEY`: the delivery key (`t:<uid>@<instant>` or `d:<date>`).
- `TWYDO_REMINDER_UID`: the task's UID, set only when the delivery has one (not the digest).

`notify.test` sets `TWYDO_REMINDER_KEY=test` and no UID. Documented in `docs/config.md` under
`command`. No protocol change.

## 6. Daemon change: the reminder error on the phone

`views.get`'s result gains an optional `reminders` summary, next to `sync`, built from the same
`notifier_error` the TUI shows: present only while there is an error, carrying its text
*(my call: the phone has no `status.get` and the sync summary already established this path)*.
The phone shows it as a warning banner in the style of `SyncWarning`
(`Reminders are failing: <text>`). Clients already refetch on `status.changed`, which fires when
`notifier_error` changes. The field is an optional addition: it must pass the released-schema
test and needs no new protocol generation (it stays 4); an older phone app ignores it.

## 7. The gateway

### 7.1 Push state

Kept in the gateway state directory with the same rules as `devices.json` (0600, flock,
atomic replace):

- **VAPID key pair** (P-256), created by `tdo gateway serve` on first start, together with the
  VAPID `sub`, which is the `--origin` URL *(my call: Apple requires `mailto:` or `https:`; the
  origin is already configured and is not personal contact data)*. Never rotated automatically;
  rotating would silently invalidate every subscription.
- **One push subscription per device**, stored on the device record: `endpoint`, `p256dh`,
  `auth`, the time it was saved. A new subscription replaces the old one. `revoke` removes the
  device and with it its subscription.

### 7.2 HTTP routes

All need a valid device key and role `phone`. `PUT` and `DELETE` also need an `Origin` equal to
`--origin` (a cookie-authenticated state change needs the same cross-site protection as `/ws`).
The two `GET`s change nothing and browsers send no `Origin` on a same-origin `GET`, so a missing
`Origin` is accepted there and a different one refused *(my call)*:

- `GET /push/key`: the VAPID public key (base64url), for `PushManager.subscribe`.
- `PUT /push/subscription`: body is the browser's `PushSubscription.toJSON()`; stores it.
  Idempotent. Rejects an endpoint that is not `https:` *(my call: push services are always
  HTTPS, and this keeps the gateway from being pointed at internal hosts)*.
- `DELETE /push/subscription`: removes this device's subscription.
- `GET /push/subscription`: whether this device has one stored (so the app can show the
  setting truthfully after a reinstall or a revoke and re-pair). *Not built: the app re-PUTs on
  every start (§8.2), so nothing calls it.*

Under `--insecure` the routes answer 404 and the app hides the setting.

### 7.3 `tdo gateway push <title> <body>`

Run by the daemon, not by people (documented, hidden from the short help *(my call)*). It:

1. Reads the push state under the lock (`--state-dir` / `TWYDO_GATEWAY_STATE` / the default,
   the same resolution as `serve`). It does not need `serve` to be running.
2. Builds the payload `{"title", "body", "key", "uid"}` from its arguments and the environment
   (§5), encrypts it per RFC 8291 (`aes128gcm`) for each subscription, signs a VAPID JWT
   (RFC 8292, ES256, `aud` = the endpoint's origin, `exp` within 12 hours), and POSTs it with
   `TTL` of 24 hours *(my call: matches the default catch-up window; a phone that is off longer
   gets the task in the next digest anyway)* and `Urgency: high`.
3. Sends to all subscriptions concurrently, within an overall deadline of 8 seconds, below the
   sink's 10-second kill *(my call)*.
4. Sorts each answer *(my call)*:
   - **accepted:** 2xx;
   - **dead:** `404`, `410` (gone), and `400`, `401`, `403` (the subscription or the VAPID key it
     was made with can never work). Removed under the lock and logged with the status. The app
     re-subscribes on its next start (§8.2); until then, if no other phone accepted, the next
     run fails with `no phone has reminders turned on`, so the loss is visible, never silent;
   - **transient:** `413` (our payload, a bug, not the phone's fault), `429`, other `5xx`,
     network errors and the deadline. The subscription is kept.
5. **Exit status:** 0 when no answer was transient and at least one was accepted. Otherwise
   non-zero with a one-line reason on stderr, which becomes `notifier_error`. A retry sends to
   every remaining subscription again; the command keeps no per-phone delivery memory
   *(my call: the sink has none, and the duplicate is the accepted cost)*. So:
   - no subscription at all (`no phone has reminders turned on`): failure, retried, visible;
     that is right, because the server is configured to push and nothing receives it;
   - one of two phones failing: failure, retried to both; the phone that already got it gets
     it again, and the notification `tag` (§8) replaces it in place rather than stacking.
6. Logs to stderr the device name, the endpoint's host and the status code; never the title,
   body or full endpoint URL (the endpoint path is a bearer token for that phone).

Crate choice is the builder's, with two constraints: pure-Rust TLS (no OpenSSL, the release
builds and the Docker image stay as they are) and a maintained RFC 8291/8292 implementation or
the primitives (`p256`, `hkdf`, `aes-gcm`) rather than hand-written crypto beyond the RFC's
glue. The binary-size increase is reported in the PR.

### 7.4 Privacy

The payload is encrypted end to end between the gateway and the phone; the push service sees
only ciphertext, its size, the TTL and the urgency. This satisfies the MVP design's note that
payloads must not leak titles to the push service. Payloads are padded to a fixed bucket
*(my call, e.g. 1 KiB)* so their length does not reveal the title's length; a body that does
not fit is shortened before encryption.

## 8. The PWA

### 8.1 Service worker

`sw.js` gains:

- `push`: parse the payload and `showNotification(title, {body, tag: key, data: {uid}})`.
  Every push shows a notification (iOS revokes push for a site that receives one silently). A
  payload it cannot parse shows `Twydo reminder` with no body.
- `notificationclick`: close it. Target is `/#/task/<uid>` when the data has a uid, else `/`
  (Today). If an app window is open, focus it and `postMessage` the target; the app sets its hash
  (no reload, so a form being typed into is not thrown away by a navigation; the app's existing
  routing handles `#/task/<uid>`). Otherwise `clients.openWindow(target)` *(my call)*.
- Notifications are shown with `renotify: false`, so a retried duplicate that replaces one by
  `tag` should not alert again where the platform honours it (not guaranteed on iOS; a second
  alert is the accepted duplicate).

### 8.2 Turning reminders on

A **Reminders** row at the bottom of the Views tab, with an on/off switch *(my call: the app has
no settings screen and the Views tab is the place for things that are not a list of tasks)*.

- Shown only when the browser supports push and the app runs standalone (on iOS push exists
  only for the home-screen app). In a browser tab it reads `Add Twydo to your Home Screen to get
  reminders`.
- Turning it on, from the tap itself (iOS requires a user gesture): `Notification.requestPermission`,
  `GET /push/key`, `pushManager.subscribe({userVisibleOnly: true, applicationServerKey})`,
  `PUT /push/subscription`. Turning it off: `unsubscribe` and `DELETE`.
- Permission denied: the switch stays off with `Allow notifications for Twydo in Settings`.
- **On every app start**, when permission is granted, the app re-reads its subscription and
  `PUT`s it again (subscriptions can change or expire, and iOS lacks
  `pushsubscriptionchange`). If the server has none for this device (a re-pair, a removal after
  `410`) but the browser has one, the PUT restores it.
- The `reminders` banner (§6) shows on Inbox and Today like the sync warning.

## 9. Tests

- **Daemon:** `CommandSink` passes `TWYDO_REMINDER_KEY`/`UID` (recording command); `views.get`
  carries `reminders` exactly while `notifier_error` is set; schema test.
- **Gateway, pure:** payload building and padding; encryption against the RFC 8291 test vector;
  VAPID JWT claims and signature verification; the answer sorting and exit-status rule over
  combinations of 2xx, 400/401/403/404/410, 413, 429, 5xx, timeout and no subscriptions.
- **Gateway, HTTP:** the push routes refuse no key, the wrong role, a wrong `Origin`, a missing
  `Origin` on `PUT`/`DELETE`, a non-`https` endpoint; `revoke` removes the subscription; `--insecure` answers 404.
- **`tdo gateway push` end to end:** against a local fake push service over TLS (test CA), the
  fake decrypts with the subscription's private key and checks the payload and VAPID header;
  gone subscriptions are removed; a hung service is cut at the deadline with a non-zero exit.
- **End to end with the daemon:** a test daemon with `command = ["tdo", "gateway", "push"]` and
  the fake push service delivers a due trigger, records it only on success, and surfaces the
  error in `views.get` when the service fails.
- **PWA:** unit tests for the subscription flow with a stubbed `PushManager`; the service
  worker's `push` and `notificationclick` handlers with stubbed `registration` and `clients`.
- **Manual, needs the user's approval:** one round on the user's iPhone against the server:
  turn reminders on, `tdo notify test` on the server, then one synthetic task with an alert in
  the isolated test list; check the notification, the tap into the task, and that a revoked
  device stops receiving. Only after that does the user turn 2Do's notifications off.

## 10. Documentation

`docs/gateway.md` (turning reminders on, the server's `[notifications] command`, HTTPS needed,
iOS 16.4+ home-screen app), `docs/config.md` (`TWYDO_REMINDER_KEY`/`UID` for commands),
`docs/protocol.md` and the schema (`reminders` in `views.get`), `docs/architecture.md`, and the
roadmap paragraph in `AGENTS.md`.

## 11. Left to the builder

Crate choice within §7.3's constraints; the padding bucket; exact wording; how the app stores
"the user turned this off" so a restart does not re-subscribe; module and type names; the
plan's task split.
