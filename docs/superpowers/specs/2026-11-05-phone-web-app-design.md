# Twydo on the phone (self-hosted web app): design

Date: 2026-11-05. Status: draft for review. No build date is set; this records the design so it
can be planned when the work starts.

This design covers a phone companion to `tdo`: a small web server that runs next to the user's
CalDAV data and a web app installed on the phone's home screen. The main spec
([2026-10-31-terminal-tasks-design.md](2026-10-31-terminal-tasks-design.md)) still governs
everything not said here. Where the two differ, this document records the user's newer decision;
see [Changes to the main spec](#changes-to-the-main-spec).

Everything here is the user's decision or follows from the existing project, except points marked
*(my call)*: those are proposals the builder may refine with a reason.

## 1. Intent

The terminal stays the place where tasks are planned, edited and reshaped. The phone covers two
moments away from the desk:

1. **Capture.** Something comes to mind; the user opens the app, types it, and it lands in
   Inbox to be sorted out later in the terminal. Often it is an idea, so a note matters as much
   as the title. Dates and lists are set later, in the terminal or, once it exists, through
   natural language at capture.
2. **Completing on the go.** The clearest example is a shopping trip: one list open, ticking
   items off one-handed. Completion is a must, for any task, not only shopping.

Everything else is done in `tdo`.

**Look and feel:** a modern, simple, native-looking iPhone app. It takes 2Do's structure where
that helps (date groups, list colours) and otherwise follows iOS conventions: large titles,
grouped rows, round checkboxes, a tab bar, sheets. It follows the phone's light or dark
appearance automatically.

**Why a web app and not a native one:** no App Store, no developer account, no signing; the
server can reuse `twydo-core` and the existing sync as they are, so nothing on the phone has to
speak CalDAV or keep its own copy of the data.

## 2. Goals, non-goals, anti-goals

**Goals (v1)**

- Browse Inbox, Today, All and each list, with the same membership, date groups and sorting as
  `tdo`.
- Capture a task with a title and an optional note.
- Complete any task the gate allows, and undo a completion straight after.
- Read a task's note.
- Install to the home screen and use full-screen.

**Non-goals (v1)**

- Moving an existing task to another list (the first candidate for the next version).
- Editing an existing task's title, dates or note; repeats, projects, checklists, deleting.
- Setting a due date, start date or list at capture (comes with natural language, §8).
- Reminders on the phone (§8).
- Working offline: the phone must reach the server, on the home network or over a VPN (§8).
- Resolving sync conflicts on the phone.
- Multiple users, sharing, accounts or passwords.
- A native app or App Store distribution.

**Anti-goals (failure even if it "works")**

- Anyone on the same network can read or change tasks without a paired device (§6).
- A write from the phone that 2Do or `tdo` would not read as its own, or that bypasses the gate.
- The phone app silently drops or changes data it does not display (repeats, items, metadata).
- Capture that needs more than opening the app, typing a title and tapping Add.

## 3. Constraints

- Single user, the same one as `tdo`.
- Self-hosted on any home server or NAS that can run a Linux container (§7).
- All writes go through `twydo-core`'s write path (gate, edit, store) with the same
  compatibility matrix and write mode. The contracts the main spec §11 fixes (compatibility gate,
  preservation, conflict, recovery) apply unchanged.
- The target browser is iOS Safari as a home-screen web app; other modern mobile browsers should
  work but are not the target.

## 4. Screens and behaviour

The screens below were reviewed as mockups in light and dark.

### 4.1 Tabs

A bottom tab bar with **Inbox**, **Today**, **All** and **Lists**. Inbox, Today and All show the
matching `tdo` views. Lists shows every list with its colour and open count, grouped as in `tdo`;
tapping one opens it with a back button.

Inbox is the list `default_list` names in the server's config, else the list named Inbox, else
the first list (the same rule as the TUI, [config.md](../../config.md)).

### 4.2 A task list (Inbox, Today, All, a list)

- A large title with the open count.
- Tasks in rounded grouped sections, one section per date group (Today, Tomorrow, No date, …,
  as `views` produces them). A list's own screen tints its title and checkboxes in the list
  colour.
- Each row: a round checkbox, the title, and one small line under it when there is something to
  say: the due (in red when due or overdue, with the time if it has one), `Starts <day>` for a
  future start date *(my call: such tasks stay in Inbox, following the `tdo` views)*, and a
  small marker when the task has a note.
- Tasks the phone cannot edit (repeats, projects, checklists and their items) still appear, with
  a small marker for their kind, and can be completed where the gate allows *(my call)*.
- Pull to refresh.
- Tapping a row opens a read-only detail sheet: title, dates, list, note *(my call)*.

### 4.3 Completing and undo

- Tapping the checkbox completes the task at once. The row turns grey and checked; an
  **Undo** banner (`Completed "…"`, Undo) stays for a few seconds.
- Undo of a write still waiting in the queue (§4.5) just removes it from the queue. Undo of an
  applied write uses the store's undo for that journal entry, so it does what `u` does in the
  TUI, including for repeating tasks ([tasks.md](../../tasks.md)). If the undo is refused (the
  task changed in the meantime), the banner says why.
- If the gate refuses a completion, the checkbox does not tick and the reason is shown, worded
  as `tdo` words it.
- Completed tasks leave the list on the next refresh, as in `tdo`.

### 4.4 Quick capture

- A floating `+` button opens a sheet with the keyboard already up and the cursor in the title.
- Below the title, a note field (multi-line). Nothing else: no date, start or list fields in v1.
  Those come later through natural language typed into the same field (§8).
- The task goes to the list on screen, else Inbox. Capture from **Today** works like `o` in the
  TUI's Today view: the task goes to Inbox due today, so it appears on the screen the user is
  looking at *(my call)*.
- **Add** saves through the normal write path and closes the sheet; **Cancel** discards. An empty
  title cannot be added.
- If the new task went to a list other than the one on screen, a short banner says where
  (`Added "…" to Inbox`).

### 4.5 Writes are queued, never blocked by sync

The phone never waits for a sync. A write (create, complete, undo) goes into a write queue and
the phone shows its result at once. A gate refusal still comes back immediately (§4.3); a write
the store later refuses comes back on the phone's next refresh, and the row returns to its
previous state. While a write is queued, its row may show a small pending marker *(my call)*.
The phone cannot stop a sync; a stuck vdirsyncer is ended by the sync runner's own timeout.

**The write queue belongs in `twydo-core`,** next to the store and the journal, so the TUI can
use it now, before any server exists, and the server reuses it later. High-level view:

- **Queueing:** when a sync holds the vdir lock, a write is checked by the gate against the
  current snapshot and saved to a queue in the state directory; the caller returns at once.
- **Applying:** whoever takes the lock next applies the queue in order through the normal write
  path: usually the sync runner as soon as vdirsyncer finishes, then syncs again; otherwise a
  `tdo` process or the server. A queued write is never left behind when the TUI quits.
- **Safety:** each queued write records the bytes of the files it was made against, as the store
  already checks, so a write whose files the sync changed is refused with the store's reason
  instead of overwriting the change.
- **Several writers:** the TUI, `tdo op` and the server may queue at the same time (for example
  one file per queued write).
- **Showing pending writes:** views lay queued writes over the snapshot, using the pure `edit`
  functions, so a completed task does not flip back until the queue is applied.
- **Undo** of a queued write removes it from the queue.

For the TUI this means an edit made during a sync is shown as pending and applied when the sync
ends, instead of being refused or waiting; it covers much of what issue #10 works around.

**This needs its own design before implementation.** Open points include the on-disk format and
its relation to the journal, ordering and refusal reporting across processes, how the overlay
interacts with family operations and undo, and what `tdo op` and the TUI show for a refused
queued write.

### 4.6 Errors and connection

- If the server cannot be reached, the app says so plainly and offers to retry; it does not show
  stale data as if current.
- Tasks with a sync conflict show a marker; the detail sheet says the conflict must be resolved
  in `tdo` on the server *(my call)*.

## 5. Architecture

```
phone (home-screen web app) ──HTTPS──▶ reverse proxy (TLS) ──HTTP──▶ Twydo web server
                                                                       │
                                                         twydo-core ── vdir ◀── vdirsyncer ──▶ CalDAV
```

- **The server** is a Rust HTTP server built on `twydo-core`. It reads the vdir into a snapshot,
  serves views as JSON, and applies writes as `Op`s (`Op::Create` with a plain `NewTask`,
  `Op::Complete`, and the store's undo) through the write queue (§4.5).
- **Sync** uses the existing runner around vdirsyncer: after the write queue drains, on a timer,
  and when the phone pulls to refresh (debounced). Conflicts go to the existing conflict store.
- **The web app** (HTML, CSS, a small amount of JavaScript) is served by the same server, so
  there is one container and one origin. It is a home-screen web app: a manifest, icons, a
  standalone display mode, and a service worker only as far as installation needs it in v1.
- **The phone's state** is the device key cookie and nothing else.

The server is its own Twydo installation: its own vdir, `config.toml`, state directory and
vdirsyncer configuration pointing at the same CalDAV server. It and the user's desktop `tdo`
meet only through CalDAV, the same way 2Do does *(my call)*.

### 5.1 The HTTP API

The API is contract-first: an OpenAPI document kept in this repository, linted in CI, and the
server is built or validated against it. It follows the
[homelab-api-spec API guidelines](https://github.com/bwilczynski/homelab-api-spec/blob/main/API_GUIDELINES.md):
plural kebab-case resources, `POST …/{id}:action` for actions
such as complete and undo, `camelCase` JSON, collections under `items` and unpaginated, null
fields omitted, RFC 9457 Problem JSON for every error (gate and store refusals carry their
reason and a machine-readable `type`), an `Idempotency-Key` header on writes so a retry from the
phone never applies a write twice, list and detail schemas, and the spec-level lint invariants.
The guidelines' tooling (Spectral, Redocly, oasdiff breaking-change checks) comes with them.

The spec lives here, not in the homelab-api-spec repository. Where the guidelines describe that
API rather than API design in general, Twydo adapts them *(my call)*:

- **No groups:** Twydo is a single-domain API, so resources sit at the top level (`/tasks`,
  `/tasks/{id}:complete`, `/lists`) instead of under `/{group}/`.
- **Authentication** is the device key of §6, as a cookie for the web app; the same key may be
  sent as a bearer token by other HTTP clients. No OAuth or scopes in v1: one user, full
  access.
- **Versioning** follows Twydo's releases (release-plz and conventional commits), not
  semantic-release.

## 6. Device pairing and security

- **Pairing:** an admin command on the server (run with `docker exec` in a container) creates a
  one-time code for a named device, valid for 10 minutes. The phone opens the app, sees only the
  pairing screen, and enters the code. The server answers with a long random device key, kept by
  the phone as a cookie. There is no other login.
- **Every other request needs a valid device key,** reads included. Without one, the server
  serves only the pairing screen and its assets.
- **Admin commands** list paired devices (name, paired at, last seen) and remove one.
- **Brute force:** each code allows a small number of wrong attempts; after that it is void and a
  new one must be made.
- **Storage:** the server keeps only a hash of each device key, in its state directory.
- **Cookie:** `HttpOnly`, `SameSite=Strict`, and `Secure` by default, so the key is only ever
  sent over HTTPS. A documented setting turns `Secure` off for someone who knowingly runs
  without TLS *(my call)*.
- **Writes are logged** with the device name *(my call)*.

## 7. Deployment (generic, documented in `docs/`)

- The project publishes a container image (Linux, x86_64 and arm64) holding the server and
  vdirsyncer, and an example `docker-compose.yaml` with an env sample for the CalDAV URL and
  credentials.
- The image goes to the GitHub Container Registry (`ghcr.io`), pushed with the workflow's own
  token and built from the same release tags as `tdo`. It pins vdirsyncer to the version Twydo is
  tested with (0.21.0 today, [compatibility.md](../../compatibility.md)) *(my call)*.
- The docs recommend a reverse proxy that terminates TLS, and binding the server's port to the
  host only (`127.0.0.1:<port>:<port>`) so the plain-HTTP port is not reachable from the network.
  TLS itself is not built into the server.
- Data (vdir, state, config) lives in one mounted volume; logs go to stdout.
- The docs show pairing, listing and removing devices, and adding the app to the home screen.
- Nothing in the repository describes a particular user's network, hosts or domains.

## 8. Later versions (direction, not designed in detail)

In rough order; each stands on its own.

1. **Moving a task to another list** (`Op::Move`), from the detail sheet or a swipe.
2. **Reminders:** the notifier (Plan 2) runs on the server and sends Web Push to paired devices.
   iOS supports Web Push for home-screen web apps.
3. **Offline reading:** the phone keeps the last snapshot and computes views locally with the
   pure parts of `twydo-core` compiled to WebAssembly (`ical`, `model`, `views`, `dateinput`,
   `repeatinput`, `edit`, `gate`, which do no file, process or clock access). v1 must keep them
   that way.
4. **Offline edit queue:** the write queue of §4.5, held on the phone while it is offline. Edits
   are checked by `gate`, shown as pending, and sent in order on reconnect; one the store refuses
   is offered to re-apply or discard. The server stays the only place data lives.
5. **Natural-language capture:** due, start, list and repeat typed into the title field
   (`call the plumber fri 5pm`), parsed by the server with `twydo_core::dateinput` and the repeat
   parser, with a preview before Add, as the main spec plans for the TUI.
6. More editing (title, dates, note), checklist items.

### 8.1 One Twydo server, several front ends

The user sees the server as more than the phone's backend: a Twydo server that can also run on
the desktop. Ideas recorded for later, none of them in v1:

- **`tdo` as a client** of a local server over a Unix socket, instead of reading and writing the
  vdir itself.
- **Background work in one place:** the server runs sync (and later the notifier) as a long-lived
  process, instead of timers and one-shot runs.
- **An MCP server,** so AI agents can read and change tasks through the same gate and write path.

So the phone's HTTP API should be one front end over a transport-neutral core of the server
(views, `Op`s, the write queue, undo, sync status, device or client identity), so that a
Unix-socket front end for `tdo` and an MCP front end can be added later without reworking it.

The front ends need not share an API. The REST API of §5.1 is shaped for the web app; the
Unix-socket interface for `tdo` should follow what `tdo` does (snapshots and views, `Op`s,
undo and redo, the queue, sync status and conflicts) rather than mirror REST resources. Access to
it is guarded by the socket's file permissions, not by device keys.

**Decide before the notifier.** Whether `tdo` gets a local server must be decided before the
notifier (Plan 2) is built, whichever comes first, this app or the notifier. A long-running
server is the natural home for reminders, on the desktop and as Web Push to the phone. A notifier
built as its own timer or daemon first would have to be moved into the server later.

## 9. Tests

- **Server:** API tests against a fixture vdir (synthetic data, as in `tools/seed_fixture.py`):
  views match `tdo`'s for the same snapshot and time; create, complete and undo produce the same
  files as `tdo op`; gate refusals come back with their reasons; writes made during a sync are
  queued and applied after it, in order, and one whose files the sync changed is refused with
  the store's reason; a write repeated with the same `Idempotency-Key` is applied once; queued
  writes survive a server restart; errors are Problem JSON; requests without a device key
  are refused; pairing codes expire, are single-use and stop after the attempt limit; removed
  devices are refused.
- **Web app:** a small set of browser tests for capture, complete, undo and pairing in light and
  dark, at phone size.
- **Interop:** in the test setup, a task captured and one completed from the phone are read
  correctly by 2Do on the iPhone and by `tdo`, recorded like the earlier phone sessions in
  `docs/reference/phase0/`. Only with the user's approval, as for every device round.

## 10. Left to the builder

- How the server is packaged and run (a `tdo` subcommand, a separate binary in the workspace,
  a daemon `tdo` starts on demand, or something else), with §8.1 in mind.
- The HTTP framework, the exact endpoints and schemas within §5.1, and whether the app uses a
  frontend framework or plain JavaScript, as long as the assets are embedded in the server and
  the result looks native.
- The sync timer interval and how the phone learns about changes (polling on focus is enough
  for v1).
- Exact wording, icons and spacing, within §4.

## Changes to the main spec

- §2 non-goals list "a mobile app of our own". The user has now decided on this self-hosted web
  app; a native or App Store app remains a non-goal.
- §1 success said the 2Do iPhone app "can still be used occasionally". That stays true; this app
  is meant to take over the phone's everyday capture and completion once it exists.
- The anti-goal "a look that ignores the Omarchy theme" applies to the terminal app only.
