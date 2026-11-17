# Phone companion MVP: a PWA over the daemon's JSON-RPC

Date: 2026-11-17. Status: draft for review. Issue #39.

This design replaces the architecture of the
[phone web app design](2026-11-05-phone-web-app-design.md) (2026-11-05) now that the
[daemon](2026-11-09-daemon-design.md) exists. That document still describes the intent, the look
and the longer-term direction; where the two differ, this one wins (see
[Changes to earlier designs](#11-changes-to-earlier-designs)). The
[main spec](2026-10-31-terminal-tasks-design.md) governs everything not said here, and its §11
contracts (compatibility gate, preservation, conflict, recovery, notification) apply unchanged.

Points marked *(my call)* are proposals the builder may refine with a reason. Everything else is
the user's decision or follows from the existing code.

## 1. Intent

The terminal stays the place where tasks are planned and reshaped. The phone covers two moments
away from the desk:

1. **Capture:** a thought comes up; open the app, type a title (and maybe a note), tap Add; it
   lands in Inbox to be sorted later.
2. **Completing on the go:** tick tasks off one-handed, with an undo straight after.

The MVP is the smallest thing that does both on the user's iPhone: a home-screen web app (PWA)
with two tabs, **Today** and **Inbox**, talking to a new `tdo gateway` command that bridges the
daemon's existing JSON-RPC protocol to a WebSocket. No App Store, no native app, no WebAssembly.

The gateway is also the way in for other devices later, for example an e-ink display on the
fridge that shows Today. The MVP builds for the phone but must not need reworking for such a
device (§8).

## 2. Goals, non-goals, anti-goals

**Goals (MVP)**

- Today and Inbox, with the same membership, date groups and order as `tdo` shows them.
- Capture: title and optional note.
- Complete any task the gate allows; undo the last completion or capture from the phone.
- Read a task's note.
- Install to the home screen; full-screen; light and dark follow the phone.
- Live updates while the app is open: a change made in `tdo` or by a sync shows up without a
  manual refresh.
- Pairing: nothing is readable or writable without a paired device.

**Non-goals (MVP)**, each a later step (§10):

- Other views: All, Lists, a single list.
- Web Push reminders.
- Dates, list or repeat at capture; natural-language input.
- Editing, moving, deleting, reopening; checklist items.
- Offline reading or offline writes; WebAssembly.
- A one-shot HTTP endpoint for sleepy devices (§8).
- Resolving sync conflicts, list management, sync control on the phone.
- Multiple users, accounts, passwords.

**Anti-goals (failure even if it works)**

- The daemon listens on TCP. It stays Unix-socket only; only `tdo gateway`, run where the user
  chooses, opens a network port.
- Anyone who can reach the gateway can read or change tasks without a paired device key, or a
  page on another origin can drive a paired phone's WebSocket.
- A paired device can call methods outside its role (list deletion, journal repair, daemon
  shutdown, sync control, config reads beyond what views need).
- A write from the phone that bypasses the gate or the daemon's write path, or that 2Do or `tdo`
  would not read as its own.
- A second protocol to maintain: the gateway forwards the daemon's own methods and types; it
  does not grow its own REST resources or its own task model.
- Showing stale data as if it were current when the connection is gone.

## 3. Constraints

- Single user. The gateway runs as the same OS user as the daemon it talks to.
- Target browser: iOS Safari as a home-screen app. Other modern mobile browsers should work.
- Clients never read `config.toml`, the vdir or the state directory (daemon design D9, D13). The
  gateway is a client and obeys the same rule for task data and Twydo configuration.
- Protocol changes follow the daemon's rules: generated `schema/tdo.openrpc.json`, the
  released-schema compatibility test, open enums for values clients receive.
- No personal data in the repository: fixtures synthetic, docs free of the user's hosts,
  domains and network.

## 4. Approaches considered

| | Approach | Trade-off | Outcome |
|---|---|---|---|
| A | **Separate `tdo gateway` process**, a client of the daemon socket, bridging an allowlisted subset of JSON-RPC to WebSocket; pairing, auth and static assets live there. | One more process; the daemon stays free of network code, web dependencies and attack surface; a gateway bug cannot stop sync or writes. | **Chosen.** |
| B | A second listener inside the daemon. | Fewer processes; network, auth and HTTP code move into the process that owns the data, which today runs on plain threads with no web stack. | Rejected. |
| C | A REST/OpenAPI API with its own resources (old design §5.1). | A second contract to keep in step with the socket protocol. | Rejected: reuse JSON-RPC. |
| D | Full client in the browser: `snapshot.get` plus `twydo-core` views compiled to WebAssembly. | Instant views, offline reading; needs a crate split and a wasm pipeline, roughly doubles the MVP, and an e-ink device could not use it anyway. | Deferred (§10). The MVP keeps the PWA's data layer behind one small module so it can be swapped later. |

Thin clients (phone MVP, fridge display) ask the daemon for finished views with a new method,
`views.get`. Full clients (TUI, CLI, a future offline PWA) keep taking the snapshot and computing
views locally. The TUI does not switch to `views.get`: it needs the full snapshot for editing
and recomputes views on every keystroke.

## 5. Architecture

```
iPhone (PWA) ──HTTPS/WSS──▶ reverse proxy (TLS) ──HTTP/WS──▶ tdo gateway ──Unix socket──▶ tdo daemon
                                                                │                          │
                                                       device keys, roles,          vdir ◀─ vdirsyncer ─▶ CalDAV
                                                       embedded PWA assets
```

- **Where it runs:** on an always-on host (the user's NAS), as its own Twydo installation with its
  own vdir, `config.toml`, state directory and vdirsyncer setup pointing at the same CalDAV
  server. It meets the desktop `tdo` only through CalDAV, as 2Do does (unchanged from the old
  design). Nothing on the user's desktop opens a TCP port.
- **Daemon:** unchanged except for `views.get` (§6). It is started the usual way on that host.
  Its desktop notifier has no sink there; the server's `config.toml` switches reminders off
  *(my call; Web Push replaces it later, §10)*.
- **Gateway:** `tdo gateway`, a new subcommand, implemented in a new workspace crate
  `crates/twydo-gateway` that `tdo` links, as it links `twydo-daemon` *(my call)*. It connects to
  the daemon socket with the same client code `tdo` uses (socket resolution, `hello` with the
  instance, on-demand start, reconnect).
- **PWA:** TypeScript with Preact, built with Vite (user decision; §9.5), in `web/`. The built
  static files are embedded in the gateway binary and served from the same origin as the
  WebSocket. Node is a build-time dependency only; the phone and the NAS never run it.

## 6. Daemon change: `views.get`

A read-only method any session may call. It runs the same `twydo_core::views` functions the TUI
uses, on the published snapshot (queued edits laid over the files, as the TUI sees it), at the
daemon's current time in the configured timezone.

**Params:** `{"view": "today" | "inbox"}`. Further views (`all`, `list` with a collection id) are
added later as new values; an unknown view is refused with kind `invalid`.

**Result** (field names are illustrative; the builder fixes them in the schema):

- `epoch`, `revision`: the snapshot the view was computed from. The client uses them as
  `based_on` for `op.submit`, so writes are planned against exactly what the user saw.
- `now` (RFC 3339 with offset) and `today` (date): the instant and day the view was computed
  for. A thin client never uses its own clock to decide what is today.
- `title`, `open_count`.
- `capture`: where a task captured on this screen goes: the collection id and display name of
  Inbox, and for Today a due date of `today` (the TUI's `o` in Today). Inbox is resolved as the
  TUI resolves it (`default_list`, else the list named Inbox, else the first list).
- `sections`: the `groups` of `twydo_core::views::build_view` for that view, in its order, each
  with a stable key (the `DateGroup`, an open enum) and its label as `tdo` words it. The TUI's
  `group_title` (`crates/tdo/src/ui/tasks.rs`) moves into `twydo-core` so both use one wording
  *(my call)*. Rows stay **flat**, as `build_view` returns them, with `depth` and the parent's
  uid and title for rows nested under, or shown away from, their parent. The expansion is
  `Expansion::default()` (no client folding state in the MVP; checklists open by default in
  Today, as in the TUI) *(my call)*. A row carries what a thin client needs to draw it and
  nothing it would have to interpret with `twydo-core`:
  - `uid`, `title`, `depth`, `parent` (uid and title, when present);
  - the due (date, optional time) and whether it is due/overdue, the future start date if any;
  - `has_note`, and the note text itself *(my call: notes are short and the detail sheet needs
    them; no second call)*;
  - the task kind marker (plain, repeating, project, checklist, item);
  - the list name and colour;
  - `pending` (a queued write touches it) and `conflict` (it has a recorded conflict).

A test asserts that, for the same snapshot and time, `views.get` returns the same membership,
grouping and order as the TUI computes. The method and its result types go into
`schema/tdo.openrpc.json` and must pass the released-schema test as an addition.

*(my call)* No new event: clients refetch `views.get` on `data.changed`, and on `status.changed`
if they show conflicts or pending markers. A day rollover needs no event either: the PWA
refetches when it becomes visible and on a timer while open (§9.4).

## 7. The gateway

### 7.1 Command line

- `tdo gateway serve --listen <addr:port> --origin <https://…>`: run the server. `--listen`
  defaults to `127.0.0.1:8787` *(my call)*, so binding to a public interface is an explicit
  choice. `--origin` is the public origin the reverse proxy serves (needed for the WebSocket
  Origin check and the cookie); required unless `--insecure` is given.
- `--insecure`: for a deliberate plain-HTTP setup on a trusted network: drops the cookie's
  `Secure` flag and allows an `http://` origin. Documented as such.
- `tdo gateway pair <device-name> [--role phone|display]`: print a one-time pairing code (default
  role `phone`).
- `tdo gateway devices`: list paired devices (name, role, paired at, last seen).
- `tdo gateway revoke <device-name>`: remove a device. Takes effect within a few seconds,
  including closing its open WebSockets.

Settings come from flags and environment variables, not from `config.toml` *(my call: the
daemon owns that file and clients do not read it)*. The `--socket` / `TWYDO_SOCKET` rules for
reaching the daemon are the same as for every `tdo` command.

### 7.2 Gateway state

The gateway's own state (paired devices, pending pairing codes, their attempt counters, each
device's daemon session id) lives in a directory of its own, by default
`$XDG_STATE_HOME/twydo/gateway/`, overridable with a flag *(my call)*. It holds only hashes of
device keys and codes, never task data. Files are `0600` in a `0700` directory and written
atomically. The admin commands change this state while `serve` runs; how `serve` notices
(watching the file, polling, a small admin socket) is the builder's call, within the revocation
deadline above.

### 7.3 Pairing and authentication

- `pair` creates a code valid for 10 minutes, single-use, with at most 5 wrong attempts before it
  is void. *(Carried over from the old design §6.)*
- An unpaired browser gets only the pairing page and its assets. Submitting the code
  (`POST /pair`) returns a long random device key as a cookie: `HttpOnly`, `SameSite=Strict`,
  `Secure` (unless `--insecure`), `Path=/`, long-lived.
- Non-browser devices (the future display) send the same kind of key as
  `Authorization: Bearer <key>`. *(my call: pairing for them is the same `POST /pair` with the
  key returned in the body as well as the cookie.)*
- Every request except the pairing page, its assets and `POST /pair` needs a valid key.
- **WebSocket upgrade** (`GET /ws`): requires a valid key and, for cookie-authenticated
  requests, an `Origin` header equal to `--origin`. This blocks cross-site WebSocket hijacking,
  which `SameSite` alone does not fully cover.
- Writes are logged to stdout with the device name and method, never task titles
  *(my call; titles are personal data and logs travel)*.

### 7.4 The bridge

- There is one daemon per vdir, as always (daemon design §7.1); the gateway never starts more
  than that one (it starts it on demand only if none is running, like any `tdo` command). A
  daemon *session* is not a process: it is an id plus that client's undo/redo stacks, held in
  the one daemon's memory, as for each TUI.
- One WebSocket connection = one connection on the daemon's Unix socket (not a process), each
  with its own `hello` and event subscription *(my call: simpler than multiplexing all devices
  over one connection; fine for a handful of devices)*. On upgrade the gateway calls `hello`
  with client name `tdo-gateway`, its version, and the device's stored daemon `session`; it
  stores the session the daemon returns. So each device keeps its own undo stack (daemon D7),
  across reconnects while the daemon runs. A daemon restart loses it, as for the TUI.
- Messages are JSON-RPC 2.0, one message per WebSocket text frame, with the daemon's method
  names, params and results unchanged, apart from the rewrites below. The client never calls
  `hello`; the gateway has done it.
- **Allowlist by role.** A method or op outside the device's role is answered by the gateway
  with a JSON-RPC error using the daemon's error shape and kind `invalid` (or a new open kind
  `forbidden`, the builder's call), and never reaches the daemon.

  | Role | Methods | `op.submit` ops |
  |---|---|---|
  | `phone` | `views.get`, `events.subscribe`, `events.unsubscribe`, `op.submit`, `op.undo` | `create` (a plain `NewTask`: collection, summary, description, at most a date-only due; no parent, repeat, kind other than task), `complete` |
  | `display` | `views.get`, `events.subscribe`, `events.unsubscribe` | none |

  The role table is data in the gateway, so a new device type is a new row, not new code paths.
- **Rewrites:** the gateway overwrites `session` in `op.submit` / `op.undo` with the device's
  session, and prefixes `request_id` with the device id, so a device can never act in another's
  session or collide with another's request ids. Retrying with the same `request_id` keeps the
  daemon's "never writes twice" guarantee.
- **Responses:** the gateway strips its `request_id` prefix from results and errors before
  passing them back, so the device sees its own ids. It keeps an in-flight map per connection;
  this request/response rewriting lives in the transport-neutral core of the bridge, so a later
  `POST /rpc` reuses it.
- **Events:** clients treat events purely as "refetch" triggers. The gateway forwards an
  allowlist per role: `data.changed`, `status.changed`, `missed`, `heartbeat`. `op.finished`
  is forwarded only when its `request_id` carries this device's prefix, with the prefix
  stripped; `job.finished` and other events are dropped, so one device never sees another's ids.
  An event value the gateway does not know (open enums) is dropped *(my call)*. The gateway sends
  WebSocket pings so proxies and iOS do not drop an idle socket.
- **Daemon gone:** the gateway closes the device's WebSocket with a close reason the PWA shows
  (`daemon not running`), and reconnects to the daemon on the next upgrade.
- **Bad input:** oversized frames (above the daemon's line limit) and non-JSON are refused by
  the gateway with the usual JSON-RPC error.

### 7.5 Static assets

`GET /` serves the PWA; also `manifest.webmanifest`, icons (including `apple-touch-icon`), and a
service worker that only caches the app shell so it installs and opens quickly *(my call; it
caches no task data in the MVP)*. Assets are the Vite build output, embedded at compile time (§9.5).

## 8. Room for other devices

The MVP implements the `phone` role end to end and the `display` role in the gateway's table
(tested with a WebSocket test client, no real device). What a fridge e-ink display needs and
how this design covers it:

- **Finished views it cannot compute itself:** `views.get` (§6).
- **A key without a browser:** bearer authentication (§7.3).
- **Read-only access:** the `display` role (§7.4).
- **Wake, fetch, sleep:** a later `POST /rpc` taking one JSON-RPC request and returning one
  response over plain HTTP, with the same auth and allowlist. Not built in the MVP; the bridge
  should be structured so a request can be handled without a WebSocket (auth, allowlist and
  rewrites in one place, transports on top).

## 9. The PWA

### 9.1 Screens

Same look as the old design's §4, reduced to two tabs (native iOS feel: large title, grouped
rounded sections, round checkboxes, bottom tab bar, sheets).

```
┌───────────────────────────┐
│ Today                   5 │
│ ┌─ Overdue ─────────────┐ │
│ │ ○ Pay the invoice     │ │
│ │   Yesterday           │ │
│ └───────────────────────┘ │
│ ┌─ Today ───────────────┐ │
│ │ ○ Buy milk         📝 │ │
│ │ ○ Call the plumber    │ │
│ │   17:00               │ │
│ └───────────────────────┘ │
│                       (+) │
│ ───────────────────────── │
│   Today        Inbox      │
└───────────────────────────┘
```

- Rows: checkbox, title, one line under it for due (red when due or overdue, time if any) or
  `Starts <day>`, a note marker, a kind marker for repeating tasks, projects and checklists, a
  small pending marker while a write is queued, a conflict marker.
- Tapping a row opens a read-only sheet: title, dates, list, note; for a conflict, "resolve it in
  `tdo`".

### 9.2 Completing and undo

- Tapping the checkbox submits `complete` at once and shows the row checked and greyed, with an
  **Undo** banner for a few seconds (`Completed "…"`).
- Only one Undo banner exists at a time: a new completion or capture replaces it, because
  `op.undo` undoes the session's *last* write, not a named one *(my call; no protocol change)*.
  Earlier writes can still be undone step by step only through the TUI-like stack, which the
  MVP does not expose beyond the banner.
- Undo calls `op.undo` for the device's session: it undoes the device's last write, whether
  still queued or applied, as `u` does in the TUI (including repeating tasks). A refusal shows
  the daemon's message as is.
- A gate or store refusal unticks the row and shows the daemon's message as is.
- The row leaves the list when the next `views.get` no longer has it.

### 9.3 Capture

- A floating `+` opens a sheet with the keyboard up, cursor in Title, and a multi-line Note.
- **Add** sends `op.submit` with `create`, built from the current view's `capture`: Inbox's
  collection, plus a due of `today` on the Today tab. Empty titles cannot be added. **Cancel**
  discards.
- After Add the sheet closes and a banner says where it went (`Added "…" to Inbox`) with Undo.

### 9.4 Connection and freshness

- The PWA opens the WebSocket when shown, subscribes to events and calls `views.get`; it refetches
  on `data.changed` / `status.changed`, on `missed`, when it becomes visible again
  (`visibilitychange`), and every few minutes while open *(my call; covers day rollover)*.
- iOS closes sockets of backgrounded apps: on becoming visible the PWA reconnects and refetches
  before showing data as current.
- Disconnected: a plain "Can't reach Twydo" state with Retry; tasks are not shown as current
  (they may be shown dimmed with "last updated <time>" *(my call)*).
- Not paired / key revoked: the pairing screen.
- All data access sits behind one small module (`getView`, `submit`, `undo`, `onChange`) so a
  later WebAssembly-backed version can replace it without touching the screens.

### 9.5 Technology and build

Chosen over plain JavaScript (no checks against the protocol; hand-written DOM updates grow
messy with later screens) and a Rust/WebAssembly UI (brings back the deferred WebAssembly):

- **TypeScript + Preact + Vite** (user decision). Preact (with `@preact/signals` for state
  *(my call)*) keeps the runtime small and the dependency surface low; Vite is the dev server
  and production build, and handles WebAssembly when the offline step comes.
- **Protocol types are generated** from `schema/tdo.openrpc.json` (the method params and
  results the PWA uses) into TypeScript as part of the web build, never hand-written and never
  committed *(my call; the generator tool is the builder's call)*. The JSON-RPC client in the
  data module is typed by them.
- **Keep dependencies few:** Preact, signals, Vite and its Preact plugin, the type generator,
  test tools. No UI component library; the iOS look is hand-made CSS. `package-lock.json` is
  committed; `npm ci` in CI. Dependabot (or the repo's existing update tooling) covers `web/`.
- **Embedding:** `npm run build` writes `web/dist/`; the gateway embeds it at compile time. When
  `web/dist/` is missing, the gateway still compiles and serves a placeholder page saying the
  PWA was not built *(my call)*, so `cargo build` and `cargo test` for the rest of the workspace
  never need Node. Release and Docker builds run the web build first, and a CI check fails a
  release whose binary carries the placeholder.
- **Development:** `tdo gateway serve --assets <dir>` serves files from disk instead of the
  embedded ones; or the Vite dev server forwards `/ws` and `/pair` to a running gateway
  (`--origin http://localhost:5173 --insecure`), so the app reloads on save.

## 10. Later, in rough order (not designed here)

1. **Lists and All** tabs: `views.get` gains `all` and `list`; a `lists` summary for the Lists
   tab.
2. **Web Push reminders:** the daemon publishes due reminders to subscribers (a new event or a
   notifier sink); the gateway holds each device's push subscription and VAPID keys and sends
   the push. Needs its own design (iOS requires the home-screen app; push payloads must not leak
   titles to the push service beyond what the user accepts).
3. **`POST /rpc`** and a real display device (§8).
4. **Natural-language capture** through `input.parse`.
5. **Moving a task** to another list (`move`), more editing.
6. **Offline:** the crate split (pure `ical`, `model`, `views`, `edit`, `gate`, `dateinput`,
   `repeatinput` into a crate that builds for wasm32), the PWA as a full client caching the
   snapshot. The gate's two direct clock reads (`gate/relocate.rs`, `gate/reparent.rs`) should
   take "now" as a parameter; doing that early is cheap and keeps the pure code pure.

## 11. Changes to earlier designs

- **Phone web app design (2026-11-05):** §4.5 (write queue) is built inside the daemon and no
  longer applies as written. §5 and §5.1 (Rust HTTP server, REST/OpenAPI contract per the
  homelab guidelines, `Idempotency-Key`) are replaced by §5–§7 here: JSON-RPC over WebSocket,
  `request_id` for idempotency. §6 pairing carries over with the WebSocket Origin check and
  bearer keys added. §8.1 is decided (the daemon). Its MVP scope (Inbox, Today, All, Lists) is
  reduced to Today and Inbox. Its §7 container deployment moves to §12 below and is part of the
  MVP.
- **Daemon design §11:** the phone front end is not a second listener inside the daemon and not
  a separate REST contract; it is `tdo gateway`, a socket client, plus `views.get` in the
  protocol.

## 12. Deployment and docs

The user runs it on a NAS that runs containers, so **the container image is part of the MVP**
(user decision). The user deploys it; the plan delivers the image, the example and the docs.

- **Image:** Linux `amd64` and `arm64`, holding `tdo` (daemon and gateway) and vdirsyncer pinned
  to the version Twydo is tested with (0.21.0 today, [compatibility.md](../../compatibility.md)).
  Pushed to `ghcr.io` with the workflow's own token, built from the same release tags as `tdo`
  (old design §7). Runs as a non-root user.
- **Processes:** the daemon and the gateway run in the one container under a small init that
  reaps children and forwards signals (for example `tini`). Both log to stdout. If either exits,
  the container exits so the restart policy brings it back *(my call; how the entrypoint starts
  the two, or whether the gateway's on-demand start is used, is the builder's call)*.
  `TWYDO_SOCKET` and the XDG directories are fixed inside the image so the daemon and the gateway
  always meet.
- **Data:** one mounted volume holds `config.toml`, the vdir, Twydo's state, vdirsyncer's
  config and status, and the gateway's state (§7.2). CalDAV credentials come from the
  environment or a mounted secret file, never from the image.
- **Config on the NAS:** the same write mode and compatibility matrix as the user's desktop, so
  the phone cannot write anything `tdo` would not *(my call)*; reminders switched off (§5).
- **Admin:** pairing and devices through `docker exec <container> tdo gateway pair <name>` and
  friends.
- **Example:** a `contrib/docker-compose.yaml` and an env sample: the port bound to
  `127.0.0.1` on the host, TLS left to the user's reverse proxy, which must pass WebSocket
  upgrades. Nothing in the repository names the user's hosts, domains or network.
- **Without Docker:** the same binaries run on any Linux host (release targets already include
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`); the docs mention it briefly with a
  systemd user unit example for the gateway *(my call)*.
- **CI:** the image build runs on pull requests that touch it (build only, no push) and on
  release tags (build and push). A smoke test starts the container against a synthetic vdir and
  checks that the PWA loads and an unpaired WebSocket is refused.
- **Web build in CI and releases:** a `web` job runs `npm ci`, type generation, `tsc --noEmit`,
  unit tests and `npm run build` on pull requests. The release binaries (cargo-dist) and the
  Docker build run the web build before `cargo build`, so every released `tdo` carries the real
  PWA (§9.5).
- New and changed pages: a `docs/gateway.md` (setup, pairing, roles, security notes), the
  `views.get` section in `protocol.md`, `architecture.md`'s diagram and "The pieces",
  `README.md` index; `python3 tools/check_links.py` passes.

## 13. Tests

- **Daemon:** `views.get` for Today and Inbox matches the TUI's views for the same fixture
  snapshot and time (membership, groups, order, counts, capture target); `now`/`today` follow the
  configured timezone; queued writes show as pending; unknown view refused; schema test passes.
- **Gateway** (against an isolated test daemon: `XDG_CONFIG_HOME` and `TWYDO_SOCKET` isolated,
  stopped with `tdo daemon stop`):
  - pairing: code expiry, single use, attempt limit; revoke closes live sockets and refuses new
    ones;
  - auth: no key, a wrong key, a revoked key, a wrong `Origin` are refused; bearer works;
  - allowlist: every method and op outside a role is refused before the daemon (including a
    `create` with a repeat or parent, `delete`, `lists.delete`, `daemon.shutdown`);
  - rewrites: a forged `session` is replaced; two devices' undo stacks are separate; resending a
    `request_id` writes once;
  - capture and complete through the gateway produce the same files as `tdo op` would;
  - events reach a subscribed WebSocket; daemon gone closes the socket with a reason.
- **PWA:** a few browser tests at phone size, light and dark: pairing, Today and Inbox render,
  complete + undo, capture from both tabs, disconnected state *(my call: Playwright with WebKit,
  the engine closest to iOS Safari, against a gateway and isolated test daemon; component and
  data-layer unit tests with Vitest)*. `tsc --noEmit` against the generated protocol types runs
  in CI, so a schema change that breaks the PWA fails the build.
- **Interop on the device:** a task captured and one completed from the phone are read correctly
  by 2Do and `tdo`, recorded like earlier rounds. Only with the user's explicit approval, as for
  every device round (AGENTS.md).

## 14. Left to the builder

- The HTTP/WebSocket libraries. Preference *(my call)*: synchronous, thread-per-connection like
  the daemon (for example `tungstenite` with a small HTTP layer), unless that proves clumsy; an
  async runtime is acceptable if confined to `twydo-gateway`.
- Exact `views.get` field names and types within §6, and whether refusals use `invalid` or a new
  open `forbidden` kind.
- How `serve` learns about admin changes (§7.2), the refetch interval, ping interval.
- Icons, wording, spacing, within the old design's look.
