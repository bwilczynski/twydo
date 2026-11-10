# The Twydo daemon: one owner for data, sync and scheduling

Date: 2026-11-09 · Status: draft for review (revised after a second review, 2026-11-09; the daemon
owns the configuration, 2026-11-09)

A long-running `tdo daemon` process becomes the only owner of a vdir: it reads and writes the
task files, runs vdirsyncer, schedules syncs and (later) reminders. The TUI, `tdo op` and the
other CLI commands become clients that talk to it over a Unix socket. Inspired by
[herdr-core's protocol](https://github.com/herdrdev/herdr-core/blob/master/schema/README.md).

Contracts in the [main spec](2026-10-31-terminal-tasks-design.md) §11 (compatibility gate,
preservation, conflict, recovery and notification contracts) bind everything here. This design
refines the main spec's §5 proposals for the sync runner and the notifier, replaces the
cross-process parts of the [write queue design](2026-11-06-write-queue-design.md), and
answers the question left open in §8.1 of the
[phone web app design](2026-11-05-phone-web-app-design.md) ("decide before the notifier").

## 1. Why

Three things led here, in the user's words and order:

1. **The phone companion needs a server anyway.** The phone design already describes a Twydo
   server with several front ends (§8.1: HTTP for the phone, a Unix socket for `tdo`, MCP).
2. **The write queue grew a daemon in disguise.** Plan 2 queues edits in per-process files,
   overlays them in every process, and starts a detached batch waiter (one per vdir, held by a
   lock file) to sync later. That is a background process coordinating others through files.
3. **Too much happens in the TUI.** The TUI works well; its *code* carries coordination that
   has nothing to do with showing and editing tasks: journal recovery at startup, draining the
   queue, starting background syncs and the waiter, tracking whether a sync runs, the sync
   switch, lock waits for `:recover`, follow-up syncs after resolving a conflict, and its own
   file watcher that re-reads the whole vdir. `tdo op` repeats much of it (queueing, `--wait`
   polling, starting the waiter).

The notifier (Plan 2) is the fourth beneficiary: it needs a process that is always running,
and the daemon is that process.

**Success:** front ends hold no coordination logic; there is one writer and one scheduler per
vdir; the TUI behaves as today (or better: it updates as soon as a sync lands); the notifier
lives in the daemon instead of a minute timer; the phone server and MCP are additional front
ends over the same core.

## 2. Goals, non-goals, anti-goals

**Goals**

1. The daemon owns the vdir: every task write, every vdirsyncer run, journal recovery, the
   write queue, batching, the automatic-sync switch, conflict bookkeeping and list
   management go through it.
2. The TUI, `tdo op` and the other CLI commands are clients over a Unix socket. None of them
   opens the vdir for writing or takes the vdir lock.
3. One protocol, documented by a generated schema, guarded against accidental breaking
   changes, usable from any language (and from `socat` for debugging).
4. Clients learn about changes by events, not by watching files or polling.
5. The daemon is started automatically when a client needs it, and kept running by a
   systemd user service or launchd agent so syncs and reminders happen with the TUI closed.
6. Undo and redo stay per client session, as `u` and `Ctrl-r` work today.
7. On-disk data is unchanged: the vdir stays plain iCalendar, the state directory keeps its
   journal, conflicts, sync status and backups. Journal entries and queue items gain optional
   fields (the session and `request_id`, §6.6), the way `from_queue` was added; older entries
   stay readable.

**Non-goals (this design)**

- The notifier itself (Plan 2). This design only makes the daemon its home (§9).
- The phone's HTTP front end and the MCP front end. They are designed elsewhere and plug in
  later (§11).
- Any network transport for the socket protocol. A `tdo connect` relay over stdio (as
  herdr's `hc connect`, for `ssh nas tdo connect`) is a later possibility, not built here.
- Changing what any operation does, the compatibility gate, the rebasing rules of the write
  queue design §6.4, or the batching rule of its §7.1.
- Windows.

**Anti-goals (failure even if it works)**

- **An edit is lost:** because the daemon crashed, a client disconnected, or the daemon was
  restarted for an upgrade.
- **A reminder is missed because the daemon is not running** and nobody notices. A stopped
  daemon must be visible (the TUI says so) and restarted by the service manager.
- **The TUI hangs** waiting for a daemon that crashed or never started. It shows a clear
  message within a couple of seconds and keeps trying.
- **Two daemons write the same vdir.**
- **An upgrade silently breaks the protocol** between a new `tdo` and an old daemon, or the
  other way round.
- **Moving the cursor or switching views waits on the socket.**

## 3. Constraints

- Single user, Linux (Omarchy) and macOS (main spec §3).
- One binary: the daemon is a subcommand of `tdo`, so `mise use github:bwilczynski/twydo`
  still installs everything.
- vdirsyncer still calls `tdo conflicts hook <A> <B>` as a separate process during a sync.
  That process stays; it is a child of the daemon's own vdirsyncer run.
- The pure parts of `twydo-core` (`ical`, `model`, `views`, `dateinput`, `repeatinput`,
  `edit`, `gate`) stay free of file, process and clock access (phone design §8 item 3).
- `feat/69-list-management` (list create, edit, archive, delete, recovery through a
  vdirsyncer adapter) is expected to merge before this work starts; its operations become
  daemon methods (§7.4).

## 4. Decisions

Marked **(user)** where the user decided, **(my call)** where I did; the builder may refine
my calls with justification.

| # | Decision | Who |
|---|---|---|
| D1 | The daemon owns the data, the sync process and all related scheduling. Clients never write the vdir. | user |
| D2 | Inspired by herdr-core: JSON-RPC 2.0 over a Unix socket, one message per line; an OpenRPC schema generated from the Rust types with a staleness test and a breaking-change test against the released schema; `hello` with a protocol generation; events with a sequence number and a `missed` resync. | user (inspiration), my call (what to keep), shown to the user |
| D3 | Left out from herdr: framed mode, channels and flow control (they exist to stream terminal bytes), and opaque documents. | my call, shown to the user |
| D4 | Coarse change events: `data.changed` and `status.changed` carry only a new version number and a hint; the client re-fetches. Outcome events (`sync.*`, `op.finished`, `job.finished`) report what happened. | my call, shown to the user with examples |
| D5 | Views are computed in the client from the snapshot, with `twydo-core`'s pure view functions. The daemon serves data, not screens. | my call, shown to the user |
| D6 | Every write names the snapshot it was based on (`based_on`: epoch and revision) and carries a client-chosen `request_id` so a retry never applies twice, even across a daemon restart. | my call (per-task revs shown to the user; changed to a snapshot revision after the builder check, §6.6) |
| D7 | Undo and redo are per client session. `u` in one TUI never undoes an edit from `tdo op`, another TUI or the phone. | user |
| D8 | The daemon is `tdo daemon`, one per vdir, started on demand by any client and kept alive by a systemd/launchd unit that starts it at login. | my call |
| D12 | The service unit is the recommended setup: `tdo daemon install` / `uninstall` writes and enables it for the platform, and `tdo` warns while no unit is installed, because without one nothing runs after a reboot until `tdo` is used. | user (accepted my recommendation) |
| D9 | No direct-file fallback in clients once the migration is done (§12): a single write path. | my call, follows D1 |
| D10 | The daemon watches the vdir and `config.toml` and treats an outside change (a hand edit, a manual `vdirsyncer sync`) like a change a sync brought in. | my call |
| D11 | The write queue stays durable on disk but is only touched by the daemon; the waiter, the minute timer and the cross-process lock dance go away. | my call |
| D13 | The daemon owns the configuration too: only the daemon reads `config.toml`; clients get it in the snapshot. A client finds its daemon at **one default socket per user**; any other setup names its socket explicitly (`--socket` or `TWYDO_SOCKET`, §6.1). `hello` still checks the client meant this daemon's configuration. | user |

## 5. Architecture

```
  tdo (TUI)   tdo op / tdo <cmd>   [later: HTTP front end, MCP front end]
      │               │                         │
      └──── JSON-RPC over $XDG_RUNTIME_DIR/twydo/tdo.sock ──────┘
                              │
                      tdo daemon (one per vdir)
        ┌─────────────┬───────┴────────┬──────────────┬─────────────┐
     store +       write queue      sync scheduler   notifier      list jobs
     journal       (durable)        + vdirsyncer     (Plan 2)      (#69 adapter)
        │                                │
        └──────────── vdir ◀─────────────┘──── conflicts hook (child of vdirsyncer)
```

- **Inside the daemon** the existing `twydo-core` modules do the work as today: `store`
  (journal, atomic writes, recovery, family operations), `gate`, `edit`, `sync` (runner,
  reconcile, conflict store, status, batching), `retired` and list management. The daemon
  adds the socket server, the session table, the event bus and the scheduler.
- **One in-memory snapshot**, reloaded when the daemon itself writes, when a sync ends, or
  when the watcher sees an outside change (D10). Queued edits are laid over it in the daemon
  (write queue design §6.5); clients receive the overlaid result with `pending` markers and do
  no overlaying themselves.
- **Two lanes inside the daemon** *(changed after review)*:
  - The **vdir lane** is the one writer: applying ops, draining the queue, vdirsyncer runs and
    list jobs run there one at a time, so the vdir lock no longer coordinates processes. The
    daemon still takes the existing locks itself (the lock files stay as a guard against a
    stray second writer and against `tdo conflicts hook`), but no client ever waits on them.
  - The **control lane** never waits for the vdir lane. It answers reads, validates requests,
    runs the gate, persists queue items and publishes the overlaid snapshot, handles
    `op.get`/`job.get`, `sync.stop` (the existing stop machinery, which works without the vdir
    lock), `daemon.*` and events. So an edit made while a sync or a list job runs is checked,
    written to the queue durably and shown as pending at once (§7.3).
- **Reads never block on writes.** A client asking for the snapshot gets the latest published
  one, even while a sync runs.
- **Threads or async** is the builder's call. vdirsyncer runs are blocking and long; they must
  not stall the socket or event delivery.

### 5.1 What leaves the clients

| Today (where) | With the daemon |
|---|---|
| Journal recovery at startup (`cli::recover_at_startup`) | Daemon, before it starts serving |
| Reader lock around loads, `queue::view` overlay (`cli/mod.rs`) | Daemon; clients get the overlaid snapshot |
| Reading `config.toml` (`cli::resolve_config`) and reloading list and group preferences (`term/preferences.rs`) | Daemon (D13, §7.2); the snapshot carries the configuration |
| vdir file watcher and full reload (`term.rs`) | Daemon watcher (D10) and `data.changed` events |
| Starting the startup sync, `:sync`, `:forget-list` sync, tracking `syncing` (`term.rs`) | `sync.run`, `lists.forget` methods; `sync.*` events |
| Sync switch (`cli::sync::set_switch`) | `sync.set_auto` method; the persisted flag stays |
| `Writer`: store, queue-on-busy, undo/redo stacks, sync trigger (`write.rs`) | `op.submit`, `op.undo`, `op.redo`; per-session stacks in the daemon |
| `:recover` lock wait | `lists.recover` job |
| Follow-up sync after resolving a conflict | `conflicts.resolve` method; the daemon schedules |
| `tdo op --wait` polling the queue directory | Waiting for `op.finished` on the connection, with `op.get` after a reconnect or `missed` |
| Spawning the batch waiter (`spawn`, `spawn_background`) | Daemon scheduler (§8) |
| `twydo-sync.timer` / `app.twydo.sync.plist` running `tdo sync --background` every minute | Daemon's periodic sync (§8) |

What stays in the TUI: drawing, keys, the edit card, the inline new row, `$EDITOR` for notes,
the live previews of the date and repeat grammars, and computing views.

## 6. The protocol

### 6.1 Transport and framing

- **One default socket per user** *(D13)*: `$XDG_RUNTIME_DIR/twydo/tdo.sock` on Linux, and
  under the per-user `$TMPDIR` on macOS. **Any other setup names its socket explicitly**: the
  `--socket` flag, else the `TWYDO_SOCKET` environment variable, else the default. A test setup
  sets `TWYDO_SOCKET` next to its `XDG_CONFIG_HOME`. There is no config key for it: the client
  does not read the configuration. The directory is created `0700`, the socket `0600`; an
  existing directory must already be ours and `0700`, and a bare relative name (`tdo.sock`)
  lives in the working directory, which is checked the same way. The daemon checks the peer's
  user id (`SO_PEERCRED` / `getpeereid`) and rejects other users; the client checks the
  daemon's the same way and refuses to talk to another user's.
- **Instance identity**: which configuration a client means, known without reading any file:
  the config file path (`Config::path()`, from `XDG_CONFIG_HOME` or `HOME`) and the `--vdir`
  override if one was given, both made absolute. It does not choose the socket; `hello` sends it
  so that a client which forgot its socket setting (a test command reaching the production
  daemon) is refused with `wrong_instance` instead of reading or writing the wrong data (§6.3). A
  client never shuts down or restarts a daemon that refused it this way.
- Two sockets whose daemons resolve to the same vdir still get one daemon: the single-instance
  lock is per vdir (§7.1), so the second daemon exits and its client reports "another daemon
  serves <vdir>".
- JSON-RPC 2.0, one message per line (UTF-8, `\n`-terminated), in both directions.
  Requests may be pipelined; responses carry the request's `id`. Notifications (no `id`) are
  events.
- No framed mode or binary channels (D3). Messages are small; the largest is a full
  snapshot. A size cap per message (for example 16 MiB) protects both sides; exact value is
  the builder's.

### 6.2 Schema and versioning

- `schema/tdo.openrpc.json` in the repository describes every method's params, results and
  errors, and every event, in JSON Schema. It is **generated from the Rust types** (for
  example with `schemars`); a test fails when the checked-in file is stale.
- `schema/released.openrpc.json` holds the last released API. A test rejects changes that
  would break an existing client: removed methods, events or fields, new required inputs,
  narrowed inputs, changed error kinds, and new values or variants in what clients receive
  (results and events) unless the enum is open (below) in the released schema: opening an
  enum and adding to it in one change is still a break. Params may gain values. The release
  pipeline copies the current schema over it (release-plz already runs on release PRs; the
  step is the builder's to place).
- **Unknown values.** The protocol's own enums that clients receive are **open**, marked
  `"x-open": true` in the schema: `WriteOutcome`, `OpState`, `SyncState`, `ErrorKind`, the
  event `type` and `Cause`. A client reads a value it does not know as `Unknown` instead of
  failing to decode the message, and treats it conservatively: an unknown outcome or state
  as "not known to be applied" (ask `op.get` again later or reload), an unknown error kind
  as a refusal to show as is, an unknown event as a reason to refetch, an unknown cause as
  `outside`. So a newer daemon may add such values without a new protocol generation. The
  daemon never sends `unknown` itself, and the schema does not list it. Enums that are not
  open (the task and configuration types inside the snapshot) gain values only with a new
  generation.
- `rpc.discover` on a running daemon and `tdo daemon schema` print the same document.
- A real break needs a new **protocol generation** (an integer, starting at 1), chosen on
  purpose.

### 6.3 Connecting

```jsonc
→ {"jsonrpc":"2.0","id":1,"method":"hello",
   "params":{"client":"tdo-tui","version":"0.4.0","protocol":1,"session":null,
             "instance":{"config":"/home/user/.config/twydo/config.toml","vdir":null}}}
← {"jsonrpc":"2.0","id":1,"result":{
    "daemon":"0.4.0","protocol":1,"epoch":"d-7f3a","session":"tui-5f2c",
    "vdir":"/home/user/.local/share/twydo/vdir"}}
```

- `protocol` must match; otherwise the daemon answers with error kind `protocol` and both
  versions, and the client follows §10.3.
- `instance` is required: the instance identity the client expects (§6.1). A daemon started
  for another identity refuses with kind `wrong_instance`, also when the client was pointed at
  the socket with `--socket`, so a misdirected client can never write the wrong data
  *(added after review; identity instead of vdir path by D13)*. The result's `vdir` is the vdir
  the daemon resolved from its configuration, for messages.
- `epoch` identifies this daemon process. A change of epoch after a reconnect tells the
  client that every version number restarted and every session is gone.
- `session` is the undo/redo session (§6.7). A client that reconnects passes its previous
  session id to keep its stacks.
- A connection that has not said `hello`, or whose `hello` was refused, may only call the
  **lifecycle methods**: `hello`, `rpc.discover`, `daemon.info` and `daemon.shutdown`. Their
  names, params and results are frozen across protocol generations (the breaking-change test
  checks them with every generation), so a client of any version can always find out what is
  running and restart it (§10.3) *(added after review)*.

### 6.4 Reading data

```jsonc
→ {"jsonrpc":"2.0","id":2,"method":"events.subscribe","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{"seq":118,"revision":42,"status_rev":17}}

→ {"jsonrpc":"2.0","id":3,"method":"snapshot.get","params":{}}
← {"jsonrpc":"2.0","id":3,"result":{
    "revision":42,
    "config":{"week_start":"monday","default_list":"a1f3","morning":"09:00", "…":"…"},
    "collections":[{"id":"a1f3","name":"Inbox","color":"#4a90d9","archived":false}],
    "tasks":[{"uid":"7d21…","rev":"c9e1","collection":"a1f3",
              "summary":"Water the plants","kind":"task","status":"needs-action",
              "due":{"date":"2026-11-10"},"rrule":"FREQ=WEEKLY;BYDAY=MO","parent":null,
              "alarms":[{"at":"2026-11-10T09:00:00+01:00"}],"meta":{"…":"…"},
              "pending":false,"blocked":null,"warnings":[]}],
    "errors":[]}}

→ {"jsonrpc":"2.0","id":4,"method":"status.get","params":{}}
← {"jsonrpc":"2.0","id":4,"result":{
    "status_rev":17,
    "sync":{"state":"idle","auto":true,"configured":true,
            "last_success":"2026-11-09T14:02:11Z","next_attempt":"2026-11-09T14:17:11Z",
            "failures":0,"last_error":null,"collections":{"…":"…"}},
    "pending_edits":0,"conflicts":0,"journal":{"unfinished":0,"stuck":null},
    "refusals":[]}}
```

- **`revision`** is the version of the whole data set; it rises by one with every published
  snapshot and restarts with the epoch.
- **`rev`** is one task's version: a short hash of its file's bytes, stable across daemon
  restarts. Clients use it to tell which rows changed between snapshots; `stale` refusals
  report the current one. Writes name the snapshot `revision`, not per-task revs (§6.6).
- **The snapshot carries what views need**: tasks as the `model::Task` fields minus file
  paths and raw bytes, collections, the configuration the daemon loaded (with its warnings;
  clients have no other copy, D13), load errors, and the
  `pending` marker from the queue overlay. The client builds its `views::Snapshot` from it and
  runs today's view functions. Parent/child resolution happens in `Snapshot::new` as today,
  on the client, so it stays one function. Exact field set and naming are the builder's,
  generated from the types.
- **Status is separate from data** because it changes far more often (every sync start and
  end, every queued edit); a status change never makes a client re-fetch tasks.
- **No partial snapshots** in protocol 1. A task list is small enough to send whole; a
  `since` parameter can be added later without breaking anything.

### 6.5 Events

```jsonc
← {"jsonrpc":"2.0","method":"event","params":{"seq":119,"type":"status.changed","status_rev":18}}
← {"jsonrpc":"2.0","method":"event","params":{"seq":120,"type":"data.changed","revision":43,
   "cause":"sync","uids":["7d21…","894c…"]}}
← {"jsonrpc":"2.0","method":"event","params":{"seq":122,"type":"sync.finished",
   "result":"ok","pulled":2,"pushed":0,"conflicts":0,"status_rev":20}}
← {"jsonrpc":"2.0","method":"event","params":{"seq":124,"type":"op.finished",
   "op_id":"q-0193","request_id":"tui-5f2c-0007","outcome":"applied","journal_id":"j-0388"}}
← {"jsonrpc":"2.0","method":"heartbeat","params":{"seq":124}}
← {"jsonrpc":"2.0","method":"missed","params":{"seq":250,"revision":61,"status_rev":30}}
```

- **Change events** (D4): `data.changed {revision, cause, uids}` and
  `status.changed {status_rev}`. `cause` is one of `op`, `sync`, `outside`, `list`,
  `recovery`, `config` (a reloaded `config.toml`, §7.2); `uids` is a hint (to keep the cursor on the right row), never a substitute for
  re-fetching. A client skips a re-fetch only when it already holds the snapshot of that
  revision. A write's answer carries the new revision number but not its contents (a family
  operation can change rows the client never named), so the client re-fetches after its own
  writes too *(changed after review)*.
- **Outcome events** report something that finished, for messages and exit codes:
  `sync.started`, `sync.finished` (result `ok`, `failed`, `stopped`, with counts and
  message), `op.finished` (for a queued op: `applied`, `refused` with kind and message, or
  `satisfied`), `job.progress` (not built yet, see §7.4) / `job.finished` (list jobs, §7.4), and later
  `reminder.due` (§9). A client never needs them to stay in step with the data.
- **Ordering:** `seq` rises by one per event and restarts with the epoch. Events go to every
  subscriber, including the connection that caused them.
- **Falling behind:** each subscriber has a bounded buffer. A subscriber that overflows it
  gets one `missed` with the current versions and must re-fetch both snapshot and status;
  events with a `seq` at or below the `missed` one are discarded.
- **Heartbeat** about every 15 seconds when idle, so a client can tell a quiet daemon from a
  dead connection.
- **Outcome events can be lost** (a disconnect, a `missed`). They are never the only record:
  `op.get {op_id | request_id}` and `job.get {job_id | request_id}` return `pending` or the
  final outcome, from the queue, the journal, the refused-items store or the job record. A
  client that reconnects, or gets `missed`, while it waits for an outcome (the TUI's queued
  edits, `tdo op --wait`) asks for it instead of waiting forever *(added after review)*.
- **Refusals of queued edits** stay in `status.refusals` until a client acknowledges them
  (`refusals.ack`), whether or not their session still exists: a live session does not prove
  the message was shown. The TUI acknowledges the ones it displayed. This replaces the
  `queue/refused/` "shown" bookkeeping (write queue design §6.6) *(changed after review)*.

### 6.6 Writing

```jsonc
→ {"jsonrpc":"2.0","id":20,"method":"op.submit","params":{
    "session":"tui-5f2c","request_id":"tui-5f2c-0007",
    "based_on":{"epoch":"d-7f3a","revision":45},
    "op":{"create":{"collection":"a1f3","summary":"Call the plumber",
                    "due":{"time":"2026-11-14T17:00:00"},"kind":"task",
                    "description":null,"start":null,"parent":null,"repeat":null}}}}
← {"jsonrpc":"2.0","id":20,"result":{"outcome":"applied","op_id":"j-0389","revision":46,
    "created":{"uid":"b07e…","rev":"41d0"}}}

→ {"jsonrpc":"2.0","id":22,"method":"op.submit","params":{
    "session":"tui-5f2c","request_id":"tui-5f2c-0009",
    "based_on":{"epoch":"d-7f3a","revision":46},
    "op":{"edit":{"uid":"b07e…","changes":[{"summary":"Call the plumber about the leak"},
                                           {"due":{"date":"2026-11-15"}}]}}}}
← {"jsonrpc":"2.0","id":22,"result":{"outcome":"applied","op_id":"j-0390","revision":47}}
```

- **`op`** is `edit::Op` with its parts (`NewTask`, `Change`, `Due`, `RepeatSpec`,
  `Placement`) as they are, serialized in snake_case. Repeat rules travel as RRULE text plus
  `from_completion`; parsing typed input (`tue`, `2w`, `fri 5pm`) stays in the client so the
  edit card's preview is instant. Clients without the parsers (phone, MCP) call
  `input.parse {"date":"fri 5pm"}` / `{"repeat":"2nd tue"}`, which runs the same
  `dateinput` / `repeatinput` code *(my call)*.
- **`based_on`** names the snapshot the client saw: the daemon's `epoch` and the snapshot
  `revision`. The client does not need to know which files an operation touches; only the
  store's family logic knows that (family members on a project completion, siblings'
  DisplayOrder on `J`/`K`, reparent cleanup). **The daemon plans against that snapshot, not
  its current one**, which is exactly what happens today, where `Store::prepare` plans
  against the snapshot the TUI loaded *(changed after review)*:
  1. The daemon keeps recently published snapshots (they are immutable and share their
     unchanged parts, so keeping several is cheap; how many or for how long is the builder's
     call).
  2. It runs `Store::prepare` on the snapshot named by `based_on`. The plan holds the bytes of
     every file the client's view involved, including family members that have since been
     deleted and the absence of files that have since appeared, and the family guard is built
     from the client's membership.
  3. The commit compares those holds with the files on disk, as today: unchanged → apply;
     changed → rebase where the write queue design §6.4 and the `rebase-merge` row allow it
     (single-task ops), else refuse as `stale`. Family operations keep today's strict check.

  A `based_on` whose snapshot the daemon no longer keeps, or from another epoch, is refused as
  `stale` with "reload and try again"; the client re-fetches and the user retries. Nothing is
  ever planned against a newer snapshot than the client saw, so the §11 preservation contract
  is exactly as strong as today's in-process check.
- **`request_id`** is required on **every method that changes anything**: `op.*`,
  `journal.revert`, `journal.abandon`, `conflicts.resolve`, `sync.run`, `sync.set_auto`,
  `refusals.ack` and every `lists.*` job *(widened after review; `sync.run` added by the builder
  in plan 2, since it starts a job record)*. It is chosen by the client and globally
  unique (session id plus a counter, the session id itself being random). The daemon records
  it, with the session, in what the call produces: the journal entry, the queue item, the
  refused-items record, the cancellation record (an undo that withdrew a queued item, §6.7),
  the conflict resolution or the job record (goal 7).
- **Replay comes first.** The daemon looks the `request_id` up **before** it checks the
  session, the epoch or `based_on`. A known `request_id` returns the original outcome without
  doing anything again, also after a daemon restart (when the session and epoch are gone), for
  as long as the record is kept (`retention_days`). So a client whose call got no answer (the
  daemon died mid-call) reconnects and resends the same request, and gets either the original
  result or a fresh run, never a second one. This matters most for undo: a lost answer to
  `op.undo` resent without it would undo a second edit.
- **Outcomes** (results, because none is a failure of the call): `applied` (with `op_id` =
  journal id, new `revision`, and `created` for creates), `queued` (a sync holds the vdir;
  with `op_id` = queue id; the final word comes as `op.finished`), `satisfied` (the change is
  already there; write queue design §6.4 step 4).
- **Refusals** are JSON-RPC errors with a stable `data.kind`:

  | kind | meaning | today |
  |---|---|---|
  | `blocked` | the gate refuses; message is the gate's reason | `WriteError::Blocked` |
  | `stale` | a file the op touches changed after `based_on` and cannot be rebased; names the task and field | `WriteError::Stale` |
  | `stuck` | an unfinished journal entry blocks writes | `WriteError::Stuck` |
  | `not_found` | no task, list, backup or session with that id | |
  | `invalid` | malformed params, empty title, bad rule | `EditError` |
  | `unavailable` | the daemon is shutting down or recovering | |
  | `protocol` | protocol generation mismatch | |
  | `wrong_instance` | the daemon serves another instance identity than the client expects (§6.3) | |

  `data` also carries `uid` and, for `stale`, `current_rev`. Message wording follows today's
  messages so the TUI and `tdo op` print what they print now.

### 6.7 Undo and redo (D7)

```jsonc
→ {"jsonrpc":"2.0","id":26,"method":"op.undo","params":{"session":"tui-5f2c",
   "request_id":"tui-5f2c-0011"}}
← {"jsonrpc":"2.0","id":26,"result":{"outcome":"applied","op_id":"j-0391","revision":49,
    "undid":{"op_id":"j-0390","title":"Call the plumber about the leak"}}}
```

- The store's session undo/redo stacks and the TUI `Writer`'s title stacks move into the
  daemon, one pair per session. `op.undo` / `op.redo` take the session and a `request_id`
  (§6.6), so a resent undo never undoes a second edit.
- The rules are today's, including the write queue design §6.7: undo of a still-queued item
  removes it; undo of an applied entry while a sync runs is queued as a `revert`; refusals as
  today ("undo after a remote edit uses conflict handling").
- **Withdrawing a queued item leaves a record.** Before it removes the item, the undo writes
  `queue/cancelled/<id>.json` with the item and the undo's request. After a restart, the
  resent undo is answered from it ("already undone") instead of undoing the new session's
  last edit, and the withdrawn edit's own request answers `queued`, with `op.get` saying
  `cancelled`. If a crash cuts the undo short (the record is written, the item is still
  there), the resent undo is refused as not applied and the item drains as usual. A redo of
  a withdrawn item queues it again under a new id.
- **Reverting by id is separate.** `journal.revert {op_id}` keeps today's `tdo op undo <id>`:
  an explicit command naming a journal entry, from any session, with today's rules. It does
  not touch any session's stacks. D7 is about `u` and `Ctrl-r`, which never reach another
  session's edits *(my call, after the builder check)*.
- **Session lifetime:** a session lives while a connection holds it and for 10 minutes after
  the last one closes, so a dropped connection that comes back keeps its stacks
  *(my call)*. Stacks are in memory; a daemon restart ends every session (the same as
  quitting the TUI today). `tdo op` uses a fresh session per invocation unless given one.

### 6.8 Method list (protocol 1)

| Area | Methods |
|---|---|
| Connection | `hello`, `rpc.discover`, `events.subscribe`, `events.unsubscribe` |
| Data | `snapshot.get`, `status.get`, `input.parse` |
| Writes | `op.submit`, `op.undo`, `op.redo`, `op.get {op_id \| request_id}`, `op.check` (gate verdict without writing, today's `tdo op check`) |
| Jobs | `job.get {job_id \| request_id}` |
| Journal | `journal.list`, `journal.recover` (today's `tdo op journal`), `journal.revert {op_id}` (today's `tdo op undo <id>`), `journal.abandon {op_id}` (today's `tdo op abandon <id>`) |
| Sync | `sync.run {force_delete}`, `sync.stop`, `sync.set_auto {on}` |
| Conflicts | `conflicts.list`, `conflicts.resolve {id, keep}` |
| Refusals | `refusals.ack {ids}` |
| Lists (#69) | `lists.create`, `lists.update`, `lists.archive`, `lists.unarchive`, `lists.delete`, `lists.forget`, `lists.recover`, `lists.backups` |
| Reports | `check.report` (today's `tdo check`), `export` (today's `tdo export` data) |
| Daemon | `daemon.info` (version, protocol, uptime, pid, vdir, epoch), `daemon.shutdown {when_idle}`: lifecycle methods, frozen across generations (§6.3) |

Names and exact params are the builder's within this shape; the schema is the reference
once generated.

## 7. Behaviour in the daemon

### 7.1 Startup

In order, before accepting any request other than `hello` and `daemon.info` (which answer
`unavailable` for everything else meanwhile):

0. Read `config.toml` for its instance identity (the same rules as `tdo` uses today: the file,
   `--vdir` overriding its vdir). A missing or invalid file is a startup error, logged and
   exited with the ordinary error code; the client that started it reports it (§10.1).
1. Take the single-instance lock `<vdir-state>/daemon.lock`. If it is held, exit with a
   distinct non-zero code and a message: another daemon serves this vdir (anti-goal: two
   daemons). The service units treat that code as "do not restart" (§10.2).
2. Create the socket directory; remove a leftover socket file only after confirming no
   process answers on it.
3. **Leftover vdirsyncer.** If `<state>/sync/running.json` names a runner that is gone but
   a vdirsyncer child of it still runs, that child still holds the shared vdir lock (the
   runner hands the lock to the child on purpose, so it is kept until the child exits). The
   daemon ends it with the existing orphan path of `request_stop` (kill the recorded process
   group if it is still the recorded process, record the stop) and then waits for the lock;
   a record whose ids belong to other processes by now is removed as stale, as today
   *(added after review)*.
4. Resume incomplete list retirements, then journal recovery, as the sync runner does today.
5. Load the snapshot, lay the queue over it, start the watcher.
6. Drain the queue if the vdir is free; start the scheduler (§8), which runs the startup
   sync the TUI used to start.

### 7.2 Outside changes (D10)

- The daemon watches the vdir and `config.toml` (the TUI's existing watcher logic moves
  here, including its rule about ignoring its own reads).
- A change the daemon did not make itself, outside a sync, is reloaded and published as
  `data.changed` with cause `outside`. Writes based on the old `rev` follow §6.6 as for any
  other device's change.
- The watcher only notices outside writes; it cannot coordinate with them. A hand edit is
  covered by the commit check (§6.6) like any other device's change, but **running
  `vdirsyncer sync` by hand while the daemon runs is unsupported**: vdirsyncer does not take
  Twydo's vdir lock, so it can write a file between the daemon's check and its commit, and it
  bypasses the conflict-hook bookkeeping and batching. `docs/sync.md` says to use `tdo sync`,
  or to stop the daemon first *(changed after review)*.
- A changed `config.toml` is reloaded and published as a new snapshot with cause `config`;
  this replaces the TUI's own preferences watcher, so list and group preferences still apply
  without a restart. Settings that cannot change at run time (the vdir, the state directory,
  the write mode and the `[sync]` binding, as the TUI's watcher treats them today) keep their old
  values and are reported in `status` as needing a restart. An unreadable file keeps the last
  good configuration and is reported in `status`.

### 7.3 The write queue (D11)

- The queue keeps its on-disk form (`<vdir-state>/queue/<id>.json`, write queue design §6.2)
  so a queued edit survives a daemon crash. Only the daemon reads and writes it.
- An op is queued while the vdir lane is busy with a vdirsyncer run or a list job (§5). The
  control lane runs the gate on the client's snapshot, writes the queue item and publishes
  the overlaid snapshot with the row pending, then answers `queued`; nothing waits for the
  vdir lane. The vdir lane drains at the end of the sync or job, in id order, exactly as §6.3
  there describes *(changed after review)*.
- Origins become sessions: an item records the session and `request_id`, so its
  `op.finished` reaches the right client and a refusal whose session is gone lands in
  `status.refusals`.
- Rebasing (write queue design §6.4, plan 3) is still needed: an edit based on data a sync
  then changes. It runs inside the daemon with the same rules and the same `rebase-merge`
  compatibility row.

### 7.4 Long-running jobs

List create, update, delete, recover and forget (#69) and `sync.run` take seconds and talk to
the network. They answer at once with a `job_id`, report `job.progress` where the adapter can
*(builder decision, plan 2: no `job.progress` yet; jobs only end)*, and end with `job.finished` carrying the result or the error kind; `job.get` returns the same
outcome later (§6.5). Jobs run on the vdir lane (§5), one at a time with syncs and writes.
`:recover` therefore no longer waits on a lock or says "a sync is running; try again": it runs
after the sync.

**Edits during a list job** are queued like edits during a sync (§7.3) and drained when the
job ends. Today's collection fence (`lists::operation::ensure_idle`) still applies at drain
time: an edit of a task in a list that the job deleted or archived is refused there with
today's reason, and reaches the client as `op.finished` with `refused`
*(added after review)*.

### 7.5 Shutting down

On `SIGTERM`, `SIGINT` or `daemon.shutdown`: stop accepting writes (`unavailable`), let a
running vdirsyncer finish or stop it with the existing stop machinery after a timeout, finish
the current write, close connections, release locks, remove the socket. Queued items stay on
disk. `daemon.shutdown {when_idle:true}` waits until no sync or job runs (used for upgrades,
§10.3).

## 8. Scheduling

The daemon runs the schedules that today live in separate processes. The rules do not
change, only where they run:

- **Startup sync:** once after startup (§7.1), as the TUI does today.
- **Periodic sync:** every 60 seconds *(the current timer's interval; a config key is the
  builder's call)*, honouring the automatic-sync switch and the runner's backoff. Replaces
  `twydo-sync.timer` and `app.twydo.sync.plist`.
- **Batched sync:** the write queue design §7.1 rule (`batch_size`, `max_wait_secs`, the
  3-second pause, D14's counting) evaluated in the daemon after every applied or drained
  edit, with an in-process timer instead of the batch waiter process.
- **After resolving a conflict:** the follow-up the TUI starts today (`after_resolve`).
- **Turning automatic sync on** with unsynced edits syncs at once, as today.
- **Wall clock:** deadlines are wall-clock times, so after the laptop sleeps the daemon syncs
  on wake, as the waiter does today.
- `sync.run` (`:sync`, `tdo sync`) runs now regardless of the switch; it waits behind a
  running sync instead of failing *(my call: today the TUI says "sync already running")*.

## 9. The notifier's home

Plan 2 builds the notifier inside the daemon, with the main spec §7 delivery contract
unchanged. This replaces the main spec §5 proposal of a `tdo notify` command on a minute
timer. Consequences fixed here:

- The daemon must be running for reminders: the service unit (§10.2) is part of the
  documented setup, not an extra.
- Reminders are computed from the daemon's snapshot; delivery state stays in the state
  directory so a daemon restart does not resend (the contract already prefers a possible
  duplicate over a lost reminder).
- A `reminder.due` event lets a connected TUI show reminders too; desktop delivery
  (`notify-send`, `osascript`) does not depend on any client.

## 10. Lifecycle

### 10.1 Started on demand

Every client command connects first. If the socket is missing or refuses:

1. **If a service unit is installed** (§10.2), the client asks the service manager to start
   it (`systemctl --user start twydo.service`, `launchctl kickstart gui/<uid>/<label>`), so
   the daemon is always the service manager's process and gets restarted if it crashes.
   **Otherwise** it starts `tdo daemon --detach` the way `spawn_background` starts a sync
   today: the same executable, its own process group, stdin and stdout closed, logs to the
   state directory. The started daemon gets the client's socket path (`--socket`), its
   environment (so the same `XDG_CONFIG_HOME`) and its `--vdir`. A client knows a unit is
   installed when the unit file for its socket exists at its known path (§10.2) *(my call, after
   the builder check; no marker file since D13, because a client cannot find the state directory
   without reading the configuration)*. A detached daemon logs under the default state root
   (`$XDG_STATE_HOME/twydo/daemon-<socket-id>.log`, `<socket-id>` a short hash of the socket
   path) for the same reason. Before starting anything, a client with no config file and no
   `--vdir` fails at once with today's "no configuration" message (checking that the file exists
   is not reading it).
2. It retries the connection with a short backoff for up to about 2 seconds.
3. If it still cannot connect, the TUI starts anyway with an empty screen and a status-line
   message (`daemon not running: <reason>; retrying`), and keeps retrying in the background;
   CLI commands fail with a clear message and a distinct exit code. Neither hangs.

Two clients starting a daemon at once is harmless: the single-instance lock (§7.1) lets one
win and the other exits.

### 10.2 Kept alive

**Starting at login.** The daemon runs after a reboot only if a service unit starts it: a
systemd user service (enabled with `systemctl --user enable`) or a launchd agent with
`RunAtLoad`. Both start with the user's login session, not at boot; that is enough, because
desktop notifications need a logged-in graphical session anyway (`loginctl enable-linger` is
not needed and not set up). Without a unit, nothing syncs and no reminder fires after a reboot
until the first `tdo` command starts the daemon on demand (§10.1), which is the "missed
reminder" anti-goal. So the unit is the recommended setup (D12), not an extra.

**`tdo daemon install`** *(user, D12)*:

- Writes the unit for the current platform for the selected socket and instance (as
  everywhere: `--socket`/`TWYDO_SOCKET`, the config file and `--vdir`):
  `~/.config/systemd/user/twydo.service` on Linux, `~/Library/LaunchAgents/<label>.plist` on
  macOS. Unit and label names include the socket id when the socket is not the default, so the
  test and production setups get separate units (§10.4).
- Points the unit at a `tdo` path that follows upgrades (the mise shim or the `PATH` entry,
  never a versioned install directory; §10.3), and passes the socket and instance explicitly
  (`--socket` when not the default, the config location's environment variable when not the
  default, and `--vdir` when given).
- Enables and starts it (`systemctl --user daemon-reload`, `enable --now`;
  `launchctl bootstrap gui/<uid>`), stopping a detached on-demand daemon for that vdir first
  (`daemon.shutdown {when_idle:true}`) so the service's daemon takes over cleanly.
- Removes or disables the old `twydo-sync.timer` / `app.twydo.sync.plist` if it finds them,
  saying what it did, since the daemon now does their job.
- Is idempotent: running it again rewrites the unit (for example after the shim path moved)
  and reports that it was already installed.

`tdo daemon uninstall` disables and removes the unit; a running service
daemon is stopped. `tdo daemon status` reports whether a unit is installed, whether it is
enabled, and whether a daemon answers on the socket.

**Warning while no unit is installed** *(user, D12)*: when a client finds no unit for its
socket, the TUI shows once per run a status-line message such as `daemon won't start at login;
run tdo daemon install`, and `tdo daemon status` says the same. CLI commands stay quiet so
scripts are not noisy. A config key (which the TUI reads from the snapshot) turns the warning off for someone who deliberately runs
without a unit (exact name and wording are the builder's).

**The unit itself.** A systemd user service (`twydo.service`, `Restart=always`, with the
"another daemon serves this vdir" exit code of §7.1 listed in `RestartPreventExitStatus`) and
a launchd agent (`KeepAlive`), replacing the sync timer units. `Restart=always` covers the
clean exit of an upgrade restart (§10.3); the excluded exit code stops a restart loop when a
detached daemon already holds the lock. With the unit installed, the daemon runs
from login, so syncs and reminders happen with no terminal open. A systemd socket unit is
optional and the builder's call. The TUI shows `daemon restarted` once when the epoch
changes, so a crash does not go unnoticed.

### 10.3 Upgrades

A `mise` upgrade replaces the binary while the old daemon keeps running.

- `hello` returns the daemon's version and protocol even when it refuses the connection.
  **Only a newer client restarts an older daemon** *(my call, after the builder check)*: when
  the client's version is higher, it calls `daemon.shutdown {when_idle:true}` and reconnects;
  the service manager (§10.2) or the on-demand start (§10.1) brings up the new binary. This
  works across a protocol change because `daemon.shutdown` is a frozen lifecycle method that
  needs no successful `hello` (§6.3) *(added after review)*.
- A client older than the daemon never shuts it down. With the same protocol generation it
  just works (the breaking-change test guarantees it). With a different one it stops with a
  message: the TUI says `tdo was upgraded; restart it`, CLI commands exit with the protocol
  error. So a TUI still running the old binary after `mise` upgrades cannot bounce the daemon
  back and forth with a new `tdo op`.
- Queued edits and the journal survive the restart; undo stacks do not (as when quitting the
  TUI today).
- The service unit must start `tdo` by a path that follows upgrades (the mise shim or
  `PATH`), not a versioned install directory. `tdo daemon install` writes it that way
  (§10.2); re-running it repairs a unit whose path went stale.

### 10.4 Several vdirs

One daemon per socket, and never more than one per vdir (§7.1). The default socket serves the
default setup; another setup chooses its socket with `--socket` or `TWYDO_SOCKET` (§6.1) and its
configuration with `XDG_CONFIG_HOME` and `--vdir`, as today. A command that sets the
configuration but forgets the socket reaches the default daemon and is refused with
`wrong_instance`, so the test setup and the production setup never share a daemon by accident.
Integration tests set `TWYDO_SOCKET` for every `tdo` they run, so a test can never start or
reach a daemon at the user's default socket.

## 11. Other front ends

- **The phone server** (phone design §5) is `tdo daemon` running on the NAS with an HTTP
  front end added: the same core, a second listener speaking the OpenAPI contract of the
  phone design §5.1, mapping onto the same methods (complete → `op.submit`, undo → `op.undo`
  with a session per paired device). It stays its own installation that meets the laptop only
  through CalDAV, as decided there. HTTP views for the phone are computed in the daemon with
  the same view functions.
- **MCP** later maps tools onto the same methods; nothing in protocol 1 needs to change for
  it.
- The socket protocol and the HTTP API stay two contracts: the socket follows `tdo`'s needs
  (snapshots, `Op`s, events), the HTTP API follows the API guidelines the phone design
  adopted.

## 12. Migration and rollout

Each step ships on its own and leaves `tdo` working *(my call; the plans may split
differently)*:

1. **The daemon takes every write at once.** `tdo daemon`, the socket, `hello`, schema
   generation and tests, `snapshot.get`, `status.get`, events, the watcher, `op.*` with
   sessions, `request_id` and `based_on`, the queue inside the daemon, the startup, periodic
   and batched syncs, **and every other mutation**: conflict resolution, journal recover,
   revert and abandon, the sync switch, refusal acknowledgement and the list jobs. The TUI and
   every CLI command that changes anything become clients in this same step; the TUI's
   `Writer` and the waiter go away. From the moment the daemon owns the vdir, nothing else
   writes it, so there is no transitional period with competing writers
   *(changed after review: the journal, conflict and list writes no longer wait for a later
   step)*.
2. **Read-only commands, setup and docs.** `check`, `export` and `status` through the daemon
   (until then they keep reading the vdir directly, which is safe under today's reader lock);
   `tdo daemon install` / `uninstall` / `status`; the `contrib/` timer units replaced by
   reference copies of what `install` writes; docs rewritten. After this no client opens the
   vdir at all (D9).
3. **Plan 3 rebasing** inside the daemon (write queue design §6.4 steps 2–4 behind
   `rebase-merge`).
4. **Plan 2, the notifier**, inside the daemon.

Between steps 1 and 2 only read-only commands still open the vdir; no direct write path
remains after step 1, and none is kept as a fallback. The write-queue plan 3 is not started
on today's cross-process design.

## 13. Tests

At least:

- **Protocol:** a daemon on a temporary socket against a fixture vdir (synthetic data,
  `tools/seed_fixture.py`): hello and protocol mismatch, snapshot and status contents, every
  method's happy path and each error kind, `request_id` replay returns the first result,
  pipelined requests answer by id.
- **Schema:** the checked-in OpenRPC file is current; a removed field or new required input
  fails the breaking-change test against `released.openrpc.json`.
- **Events:** one `data.changed` per published revision; `missed` when a slow subscriber
  overflows; events after `missed` resume in order; heartbeat on idle.
- **Writes:** an op whose touched files (including family members the client did not name)
  changed after `based_on` is refused `stale` with `current_rev`; a `based_on` from an old
  epoch is refused; a `request_id` resent after `kill -9` of the daemon between commit and
  reply returns the first outcome and creates no duplicate task; queued during a sync, `op.finished`
  after the drain; a queued item survives `kill -9` of the daemon and is applied once after
  restart; undo is per session (two sessions, each undoes only its own edit); a session
  survives a reconnect within its lifetime and is gone after a daemon restart.
- **Lifecycle:** on-demand start from a client; two simultaneous starts leave one daemon; a
  stale socket file is replaced; a second daemon on the same vdir exits; `SIGTERM` during a
  sync leaves no lost edit; a newer client restarts an older daemon when idle; an older client with another protocol
  generation never shuts down a newer daemon; with a unit installed the client starts the
  daemon through the service manager, and a lock loser exits with the no-restart code.
- **Install:** `tdo daemon install` writes a unit that names a stable `tdo` path and the vdir,
  and is idempotent; `uninstall` removes it; a non-default socket gets its own unit; an existing sync timer is reported and disabled; the TUI warns once without a unit and
  not with one. Unit contents are checked as files; enabling them
  through `systemctl` / `launchctl` is behind a seam the tests replace.
- **Outside changes:** a file edited by hand is picked up and published with cause
  `outside`.
- **Configuration (D13):** a client never opens `config.toml` (its tests run with an unreadable
  config file and still work through the daemon); a changed list preference reaches the client
  as a new snapshot with cause `config`; a changed vdir or write mode is reported as needing a
  restart and not applied; `--socket` wins over `TWYDO_SOCKET`, which wins over the default; a
  client with another instance on the default socket is refused with `wrong_instance` and does
  not start, stop or restart that daemon.
- **Cases from the second review:**
  - every mutating method replays a resent `request_id`, including `op.undo` (a resent undo
    whose answer was lost does not undo a second edit) and list jobs; the replay works after a
    daemon restart, before any session or epoch check;
  - `op.get` / `job.get` return the outcome of a queued op or job whose `op.finished` /
    `job.finished` the client missed; `tdo op --wait` survives a daemon restart;
  - a refusal stays in `status.refusals` until acknowledged even while its session lives;
  - an edit sent while a sync or a list job runs is answered `queued` at once and shown
    pending, and `sync.stop` works while vdirsyncer runs;
  - a family operation planned on the client's snapshot is refused when a child that existed
    in that snapshot was deleted since (the case planning on the current snapshot would miss);
    a `based_on` the daemon no longer keeps is refused `stale`;
  - a client re-fetches after its own family write and sees the changed
    rows it did not name;
  - a client with another protocol generation can still call `daemon.info` and
    `daemon.shutdown`; a client pointed at another instance's socket gets `wrong_instance`;
  - a daemon started while an orphaned vdirsyncer still holds the vdir lock ends it through
    the orphan path and then starts normally.
- **Scheduling:** the write queue design's batching tests move to the daemon's scheduler
  (size trigger, time trigger, pause, switch off, wake after sleep), using an injectable
  clock.
- **TUI:** the existing `app_keys`, `write`, `ui_render` tests keep working against an
  in-process daemon or a fake client behind one client interface, so they need no socket
  *(my call)*; one end-to-end test drives the TUI against a real daemon.
- **Peer check:** a connection from another user id is refused (where the platform allows the
  test).

## 14. Documentation to update with the change

`docs/architecture.md` (the daemon, the protocol, the write path), `docs/sync.md` (who syncs,
the service unit instead of the timer), `docs/install.md` (service setup, upgrades),
`docs/config.md` (the sync-interval key; who reads the file), `docs/tui.md` (daemon status messages),
a new `docs/protocol.md` that introduces the schema for people writing their own clients, and
`AGENTS.md`'s progress note. `python3 tools/check_links.py` must pass.

## 15. Changes to other specs

- **Main spec §5:** the sync runner is run by the daemon, not after edits by each process or
  by a minute timer; the notifier is part of the daemon, not `tdo notify` on a timer. The §7
  delivery contract is unchanged.
- **Write queue design:** §6.1's lock-try path, §6.2's per-process origins, §6.5's overlay in
  every process, §7.2's batch waiter and §8's `ended` file are replaced by §7.3, §8 and the
  events here. The queue format, draining order, rebasing rules, batching rule, the switch
  and the status-line content stay.
- **Phone design §8.1:** decided as described in §11 here.

## 16. Left to the builder

Threads or async and which crates; the exact socket path rules on macOS; the socket-id hash;
message size cap; snapshot field names (generated from the types); how long superseded file
versions are kept for rebasing; the daemon's
log format and rotation; whether to offer a systemd socket unit; the periodic sync interval
key; the shape of `job.progress`; method naming details within §6.8; message wording within
today's style.
