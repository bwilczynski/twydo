# Architecture

How Twydo is put together, for contributors and for anyone who wants to know what happens to
their files. For what a task looks like on disk, see [data-model.md](data-model.md).

## The pieces

```
 iPhone (2Do) ──CalDAV──▶ CalDAV server ◀──CalDAV── vdirsyncer ──files──▶ vdir
                                                         ▲                  ▲
                                                         └─ runs it ─ tdo daemon ◀──socket── tdo (TUI, CLI)
```

- **The vdir** is the source of truth on your machine: a folder with one subfolder per list
  and one `.ics` file per task, kept in step with the CalDAV server by
  [vdirsyncer](https://vdirsyncer.pimutils.org/). Twydo never talks CalDAV itself.
- **The daemon** (`tdo daemon`) owns the vdir: it reads it, writes changes back as files, runs
  vdirsyncer with recovery before and conflict handling after ([sync.md](sync.md)), and
  decides when to sync. See [The daemon](#the-daemon).
- **`tdo`** is its client: the TUI, `tdo op`, `tdo sync` and `tdo conflicts resolve` ask the
  daemon instead of touching the vdir themselves.
- **2Do on the iPhone** syncs with the same server. Twydo writes every change the way 2Do itself
  writes it, so the phone reads Twydo's changes as its own ([compatibility.md](compatibility.md)).

Twydo keeps its own state (the operation journal, sync status, conflicts) outside the vdir, under
`state_dir` ([config.md](config.md)). Task data stays iCalendar; nothing in the state directory
is an alternative active task database. Recovery backups deliberately retain task copies.

Explicit list retirement also keeps inspectable backups under `retired/<id>/`. It publishes
the complete original files and journal/conflict evidence durably before removing the local
collection, then requests ordinary vdirsyncer rediscovery. An incomplete retirement resumes
under the exclusive vdir lock before journal recovery. Journal entries entirely in that
collection are abandoned locally, with originals kept in the backup; entries spanning lists
retain their surviving writes without claiming archived writes synced. Archived conflicts
cannot write into the old collection through the normal resolver.

Recovery plans the archive's local snapshot using existing gated moves, retains destination
writes only, and journals them together. The backup records the chosen destination and exact
operation/identity mapping before it applies, so retries reuse identities and never duplicate
the import. Known family/history references follow the remapped identities. Backups survive
normal journal/conflict retention. User-facing commands are in [tui.md](tui.md#commands).

## The daemon

One daemon serves one vdir. The TUI and the commands that write or sync start it when they
find none (`tdo daemon --detach`, in a session of its own; its log is
`$XDG_STATE_HOME/twydo/daemon-<socket-id>.log`, mode 0600), so nothing has to be set up.
`tdo daemon stop` stops it once it is idle; the next client starts it again. A client that
finds the daemon gone (stopped, crashed) shows `daemon not running: <reason>; retrying`,
reconnects when it is back and says `daemon restarted`.

- **The socket.** `--socket`, else `TWYDO_SOCKET`, else `$XDG_RUNTIME_DIR/twydo/tdo.sock`
  (under `$TMPDIR` on macOS). Another setup that must not meet the usual daemon, such as a
  test setup with its own `XDG_CONFIG_HOME`, sets `TWYDO_SOCKET` as well. Both sides check
  that the other end is the same user.
- **Who reads the configuration.** Only the daemon. A client sends the path of its config file
  (and `--vdir`) in `hello`; a daemon that serves another configuration refuses it, so a
  misdirected command never reads or writes the wrong data. Changes to `vdir`, `state_dir`,
  `[writes]` and `[sync]`, and to `timezone` and `work_days`, take effect after a daemon
  restart ([config.md](config.md)); the rest is reloaded live.
- **What it does.** It holds the loaded snapshot and tells clients when it changes (an edit, a
  sync, a change made outside, a reloaded configuration). It applies every write through the
  [write path](#the-write-path), one at a time on the vdir, together with syncs and list
  operations. It syncs at startup, every 60 s, after edits (in batches), after a recorded
  conflict choice, when automatic sync is switched on and after the machine wakes
  ([sync.md](sync.md#when-edits-sync)).
- **Nothing else reads files.** No client opens the vdir, the state directory or the
  configuration. The read-only commands are methods too: `check.report` (the text of `tdo check`
  and `tdo op check`, from the snapshot with queued edits applied), `export` (the schema-v1
  document, from the vdir as last read, without queued edits), `conflicts.show` (the fields of
  the local, remote and current copies and their file paths) and `status.get`, `snapshot.get` and
  `conflicts.list` for `tdo status`. When a read-only command cannot reach a daemon because the
  vdir is missing or the configuration is invalid, the error comes from the daemon start
  (`daemon not running: …`) with exit 2.
- **Starting it.** A client that finds no daemon starts it. With a service unit installed
  (`tdo daemon install`) it asks the service manager (`systemctl --user start`, or `launchctl
  bootstrap` and `kickstart`), and falls back to a detached start if that fails. Without a unit
  it starts a detached daemon. See [install.md](install.md#start-it-at-login).
- **Outside changes.** The daemon's file watcher notices files changed behind its back (a
  vdirsyncer run, a hand edit), so a change shows up after a short delay.
- **Upgrades.** A client that is newer than the running daemon asks it to stop once it is
  idle and starts its own version. A client that is older than the daemon says
  `tdo was upgraded; restart it`.

The daemon speaks JSON-RPC over the socket, one message per line; see
[protocol.md](protocol.md) for people writing their own client.

Its protocol is described by `schema/tdo.openrpc.json`, generated from the Rust types; a test
compares it with `schema/released.openrpc.json` and fails on a change that would break a
released client. The release PR refreshes `released.openrpc.json`. The protocol's own enums that
clients receive (outcomes, operation states,
error kinds, events) are open (`"x-open": true`): a client reads a value it does not know as
`Unknown` and acts conservatively (refetches, or shows the message as is), so a newer daemon
may add values. Every other change a client could not read needs a new protocol generation
([design §6.2](superpowers/specs/2026-11-09-daemon-design.md#62-schema-and-versioning)).
The design is [here](superpowers/specs/2026-11-09-daemon-design.md).

## Crates

The repository is a Cargo workspace with three crates.

**`twydo-core`**, the library. Nothing in it draws to a terminal.

| Module | Responsibility |
|---|---|
| `ical` | A lossless iCalendar line layer: every line keeps its original bytes (folds and line endings included), so an unchanged file is written back byte for byte |
| `model` | Reading a `VTODO` into a `Task`: dates, status, parent links, repeat rules and their class, 2Do's `X-2DOAPP-METADATA` in both encodings |
| `vdir` | Loading a vdir into a snapshot (read only) |
| `views` | Today, All, Scheduled, Done, lists and smart lists: membership, date groups, sorting and counts. Pure functions of a snapshot and "now" |
| `dateinput` | The date grammar of the edit card (`fri 5pm`, `3d`, `15th`, …) |
| `repeatinput` | The typed repeat grammar (`daily`, `2w`, `mon thu`, `last day`, `2nd tue`, `after done`) and its preview line |
| `edit` | Each operation as a change to one file, the way the 2Do iPhone app makes it |
| `gate` | Whether an operation may run, and the exact reason when not ("verified mode") |
| `compat` | The compatibility matrix, `compat/matrix.toml`, built into the binary |
| `store` | Applying operations: the vdir lock, the operation journal and crash recovery, atomic file writes, family operations on projects and checklists |
| `sync` | The sync runner around vdirsyncer: output parsing, the conflict hook and store, reconciliation, status and backoff |
| `config` | `config.toml` |

**`twydo-daemon`**: the wire protocol (types and the generated schema), the server (the vdir
lane, the write queue's scheduling, sync jobs, events) and the client library.

**`tdo`**, the binary: the CLI subcommands (`cli/`), the TUI's state and keys (`app.rs`), the
edit card's state (`form.rs`), the terminal loop and its link to the daemon (`term.rs`,
`term/link.rs`), and rendering (`ui/`).

## Permission reports

`tdo check` separates data health, repeat-class evidence and effective operation
admission. It and `tdo op check <uid>` share concrete title/notes/lifecycle/delete
probes that call `gate::check` in the configured write mode. Both read under the
existing reader lock and apply `queue::view` after loading, matching the CLI write
path's view of pending changes. Startup journal recovery remains in place; reporting
does not drain the queue or schedule sync. Export and TUI loading are unchanged.

These are gate probes, not store plans: actual values, destinations, conflicts,
recovery status and stale dependencies can still affect a subsequent save.
Parameterized actions are explicitly marked as needing input. Rule-level pass
verdicts never imply that every operation is allowed.

## The write path

Every write, from a TUI key, `tdo op` or a conflict resolution, goes the same way. The client
sends the operation to the daemon, together with the snapshot revision it planned against and a
request id (a resent request gets the same answer, never a second write); the daemon runs these
steps and answers:

1. **Gate.** `gate::check` decides whether the operation is allowed for this task. It looks up
   the task's case (one-time, monthly, yearly, checklist, project, …), the matrix rows for that
   case and operation, and the verdicts for its repeat rule class. In the default `verified`
   mode, anything the matrix has not marked as passing is refused with a reason, before
   anything is written.
2. **Edit.** The `edit` functions produce the new file contents. Only the properties the
   operation is about change, plus `DTSTAMP`, `LAST-MODIFIED` and their counterparts in 2Do's
   metadata; every other line keeps its bytes.
3. **Store.** `Store::submit` prepares the operation (gate and plan, against the view with queued
   writes laid over it, see [The write queue](#the-write-queue)), then tries the exclusive vdir
   lock for 250 ms. With the lock it drains the queue and commits; without it the prepared
   operation is queued instead. Committing: under the lock, the store checks that every file it is about to
   write still holds the bytes the caller loaded (otherwise: "changed on disk; reload and try
   again"), records the operation in the journal, applies it, and marks it applied. Files the
   operation relies on but does not write are checked the same way: the parent of a new item,
   and the family members it leaves as they are. An operation whose result depends on a task's
   items (a family operation, a checklist's date change, a change of type) also rescans the
   list and is refused if an item arrived after the load.

An operation that touches a family (a project or checklist and its items) writes the parent and
every affected child in one journal entry, so it is applied completely or not at all.

## The write queue

A write that cannot get the vdir lock within 250 ms (a sync holds it) is queued instead of
refused, so `Store::submit` never reports "busy". The queue lives under `<vdir-state>/queue/`:

- `<id>.json`: a queued item. It stores the op and the journal entry planned against the
  caller's view (`writes[].before` and holds are its base, `family` is the family guard), so a
  queued create keeps the UID the view showed. After pushing, `submit` tries the lock once more
  and drains at once if it got it. Ids sort in creation order (time, per-process sequence, pid).
- `<id>.claim`: an item being applied. A drainer renames `<id>.json` to `<id>.claim` before it
  commits; undoing a queued edit removes `<id>.json` instead. Whichever filesystem call wins
  decides, and the loser backs off, so no lock is needed to cancel.
- `cancelled/<id>.json`: an item an undo withdrew, with the undo's request. It is written
  before `<id>.json` is removed (and removed again if a drainer claimed the item first), so
  the daemon can answer a resent undo after a restart. While `<id>.json` still exists the item
  counts as queued, whatever the record says.
- `refused/<id>.json`: why an item was refused (with `shown`, whether the user was told),
  pruned after `retention_days`.

Draining (oldest id first) happens in the daemon: after each write that gets the lock, in the
sync runner (after recovery and after reconcile), when a sync or list job ends, and at startup
([sync.md](sync.md#what-tdo-sync-does)). A client is told when its item was applied or
refused. Each item has the gate re-checked on the current files and its
base compared strictly, whole file by whole file; a mismatch refuses the item. A stuck journal
stops the drain and the items stay queued. An I/O failure also stops the drain without
recording a refusal: a prepared journal entry may still recover. A later drain finishes
that recovery before removing the claim and refreshes its snapshot for dependent edits.
The journal entry an item produces records
`from_queue {id, queued}`: an item named by an entry is removed, never applied twice, and
batching ages the entry by `queued`, not by the drain time.

`queue::view` lays queued writes over the loaded files while their base still matches, so the
snapshot the daemon serves shows queued edits; the TUI marks those rows ` (queued)` and counts
them in `N pending`.

## Collection operation coordination

Collection management has a separate durable fence under `<vdir-state>/lists/`:
`active` names an operation in `operations/<id>.json`. Its phases are prepared,
remote-started, remote-confirmed, local-confirmed, complete and uncertain. The record
retains the exact request and local collection bytes; it contains no credentials.
An unfinished record blocks task recovery, new task writes, queue insertion/drain and
ordinary sync until the collection service reconciles it. Existing unrelated queued
work stays pending. A coordination lock serializes marker publication with queue
insertion, so a queued write cannot slip behind the collection preflight.
If a TUI starts while that fence prevents opening its task store, the next task write
retries initialization after reconciliation clears it. An already healthy writer keeps
its session undo history.

The bundled collection helper takes the same exclusive run lock and a shared vdir lock itself.
Those locks survive the TUI's exit while the helper is still running. It validates the
bound operation and held bytes before its remote phase, and persists the receipt before
releasing the locks. Readers remain responsive during network requests; Rust local
finalization takes the exclusive vdir lock. The list card exposes explicit service actions
without enabling unverified writes in verified mode.
The helper also enforces its own operation deadline, so it releases its locks even if its
monitoring TUI has exited. Timeout retains an uncertain/prepared operation for reconciliation;
it never assumes that killing a request undoes a server mutation.

The create/edit service records the exact remote URL before a mutation and a digest of
its unexpanded connection configuration. Reconciliation uses that saved URL and rejects
changed account/storage configuration before fetching credentials. A successful remote
request is followed by a persisted local metadata before/after plan and then vdirsyncer's
own discovery-cache refresh, with missing-collection creation disabled. An interrupted
local metadata write can resume only from its recorded before/after values; held task
bytes cannot be replaced. Recovery also recognizes `.displayname.tdo-tmp` and
`.color.tdo-tmp` left before an atomic rename, only when the saved plan proves the file
was absent before the operation and contains a prefix of the intended replacement.
The helper validates these bytes without modifying them; local finalization removes
them under the exclusive vdir lock before retrying. Pre-existing scratch files,
unrelated files and changed task bytes remain protected.
Reconciliation does not repeat MKCALENDAR or PROPPATCH. If the
server's values differ, accepting them requires the explicit Keep server choice.
The metadata writer remains an unverified prototype: its read/PROPPATCH/read sequence
has a remote-writer race. The shipping metadata capability stays closed in every mode,
including test mode, until a server-enforced conditional write is implemented and proven.
Its independent `list-metadata-atomicity` edge must pass as well as the interoperability
row; promoting phone verdicts alone cannot admit a known unsafe write.
Offline protocol/reconciliation tests use a synthetic matrix override; the application
and proposed device round cannot load that override.

Deletion previews are read-only collection operations. Under the same fence, the helper
captures all returned Depth-1 resources with strong ETags, conditional GETs, SHA-256
hashes and a second matching inventory/property read. Rust publishes a private version-1
backup under `lists/backups/<id>/backup.json` with raw local files and relevant state before
releasing the fence and showing confirmation. A failed preview can be cancelled safely
without another network request. Only complete manifests appear in the collection-backup
browser; raw export checks resource/local hashes and creates a new directory outside the
vdir. This checkpoint contains no collection DELETE path or collection restore path;
server-enforced deletion protection must be demonstrated first.
The approved lifecycle probe demonstrated that a changing calendar-export ETag was
insufficient: the server ignored a stale `If-Match` on DELETE. Only that disposable
test collection was removed. All three resources, including the remote-only task, were
retained in the service backup and verified through its local raw-export API. The
isolated test sync profile was sealed; this did not enable a product DELETE path.

## The operation journal

Every write goes through the journal, so a crash at any point leaves the data either untouched
or recoverable to the complete result of the operation, exactly once.

### Where it lives

```
<state_dir>/                  e.g. $XDG_STATE_HOME/twydo/
  vdirs/
    <id>/                     one per vdir ("<state>" below)
      vdir                    identity file: the canonical vdir path and a newline
      vdir.lock               the advisory lock (see "Locking")
      journal/
        20261101T120000Z-3fa9c2d1.json   one file per operation, named <id>.json
      conflicts/              sync conflicts (see sync.md)
      sync/                   sync status, manifest, run lock and the running sync
```

Journal paths are relative to the vdir, so each vdir has its own state directory. `<id>` is the
FNV-1a 64-bit hash of the canonical vdir path (symlinks and `.` resolved), as 16 lowercase hex
digits.

- Writers create `<state>` and its identity file when missing. Readers (startup recovery,
  `tdo op journal`) create nothing: a vdir nothing has written to has no state.
- If the identity file names a different path than the vdir being opened (only a hash
  collision or manual tampering does that), Twydo refuses and names both paths. Its journal is
  never replayed; move that directory away to continue.
- A moved or renamed vdir hashes to a new `<id>` and starts with an empty state directory. Its
  old journal, including unfinished entries, stays behind and is not recovered. Before moving a
  vdir, finish or abandon its unfinished operations (`tdo op journal`, `tdo op abandon`), or move
  the old `vdirs/<id>/journal/` entries into the new directory by hand.

An operation id is `<YYYYMMDDTHHMMSSZ>-<8 hex>`: the UTC creation time and 8 random hex digits.
Ids sort chronologically, and recovery processes entries in id order.

Journal creation times preserve commit order under the exclusive vdir lock. When the clock
reading is no later than the preceding entry (including several items in one queue drain),
the next entry uses the preceding time plus one nanosecond. Pending entries are sorted by
this time, then id for older ties, so sync reconciliation and emptied-list checks use the
last committed version rather than a random id suffix. Task timestamps retain the edit time.

All files, in the journal and in the vdir, are written atomically: the bytes go to
`.<name>.tdo-tmp` in the same directory, the temporary file is fsynced and renamed over the
target, and the directory is fsynced. Removals unlink and then fsync the directory.

### Entry format (format 1)

```json
{
  "format": 1,
  "id": "20261101T120000Z-3fa9c2d1",
  "created": "2026-11-01T12:00:00Z",
  "kind": "complete",
  "uids": ["synthetic-uid-1"],
  "stage": "prepared",
  "inverse_of": null,
  "writes": [
    {
      "path": "tasks-collection/synthetic-uid-1-history.ics",
      "before": null,
      "after": "BEGIN:VCALENDAR\r\n…completed occurrence…\r\nEND:VCALENDAR\r\n"
    },
    {
      "path": "tasks-collection/synthetic-uid-1.ics",
      "before": "BEGIN:VCALENDAR\r\n…old due date…\r\nEND:VCALENDAR\r\n",
      "after": "BEGIN:VCALENDAR\r\n…next due date…\r\nEND:VCALENDAR\r\n"
    }
  ]
}
```

- `kind`: `create`, `edit`, `complete`, `reopen`, `move`, `delete`, `undo` or `redo`.
- `uids`: the task UIDs the operation targets (for a family operation: the parent, then every
  UID it created or touched).
- `stage`: `prepared`, `applied` or `abandoned`.
- `inverse_of`: for `undo` and `redo`, the id of the operation being reversed.
- `writes`, in application order. `path` is relative to the vdir (`<list>/<file>.ics`);
  absolute paths and `..` are rejected. `before` and `after` are the **full file contents**;
  `null` means the file is absent, so `before: null` creates a file and `after: null` deletes
  one.
- `synced` (optional): when a sync confirmed that the server holds the result.
- `holds` (optional): read dependencies as `[relative path, full contents]` pairs.
  Recurring-checklist item edits retain parent bytes; individual checklist/project item
  deletion retains parent and sibling bytes. Ordering and item creation also retain their
  family dependencies. Apply, recovery, undo and redo require these bytes to remain
  unchanged. They are never written or treated as sync mutations. Older entries
  without this field have no durable read dependencies.
- `family` (optional): the collection, parent UID and loaded family paths for item
  creation, ordering or deletion, including whole-family deletion. Under the vdir lock, a rescan rejects
  newly joined members before applying, undoing, redoing or recovering the operation.
  Byte dependencies guard original members; the operation's own created/deleted path
  is included and may be present/absent for idempotent recovery. Item creation records
  sibling byte dependencies because its append position depends on the loaded sibling
  set. The guard is retained by inverse entries and absent from older journal entries.
  Membership is checked on parsed, unfolded calendar links, so folding cannot hide a
  newly joined item. New guards also record `parent_aliases`: the RFC UID and internal JSON
  identity. The scanner checks JSON `parentuid` as well as standard links, so a missing
  `RELATED-TO` cannot hide a new reference after the parent file is removed. Older guards
  without aliases still decode and retain their standard UID checks. An unreadable calendar
  resource outside the recorded member paths
  cannot safely be excluded from the family and makes the operation refuse.

Writes are ordered so that a completed occurrence is created before its live task advances: if
an operation stops halfway, there is never a live task that has moved on without its history
copy on disk. Family operations write creates first, then updates, then removals.

### Protocol

A writer performs one operation as:

1. take the exclusive vdir lock;
2. write the entry with stage `prepared`, durably, before the vdir is touched;
3. apply the writes, in order;
4. set the stage to `applied`;
5. release the lock.

Applying a write compares the file's current bytes:

| Current content | Action |
|---|---|
| equals `after` | skip (already applied) |
| equals `before` | write `after` atomically, or remove the file if `after` is null |
| anything else | conflict |

Every write is checked before any is made, so a conflict leaves the vdir untouched. Because
applied writes are skipped, applying an entry twice has the same effect as applying it once.

### Recovery

Recovery runs in the daemon at startup, before every sync, and before every write, and nowhere else:

- with no journal directory, it does nothing and creates nothing;
- otherwise, under the exclusive lock, every `prepared` entry is rolled forward and marked
  `applied`; `applied` and `abandoned` entries are left alone;
- a `prepared` entry that conflicts (a file holds neither `before` nor `after`, for example
  because a sync brought in another device's change), has an invalid path, or cannot be parsed
  stays as it is and is reported as **stuck**, with the reason. Nothing is written for it.
  `tdo op journal` lists it; `tdo op abandon <id>` gives up on it;
- an I/O error stops recovery; the entry stays `prepared` and is retried next time.

Recovery is idempotent: a second run finds nothing to apply and changes nothing.

At the start of one of those commands, recovery does not wait for a running sync: when a sync
holds the vdir lock, it is skipped, so the command runs at once. Nothing is lost by that. The sync recovered before
vdirsyncer started, and the next write recovers under the lock before it changes anything.

### Undo, redo and retention

`u` and `Ctrl-r` in the TUI (and `tdo op undo <id>`) write a new journal entry that reverses an
earlier one, under the same "file must still hold the expected bytes" rule: an undo after a
sync brought in someone else's change to the same task is refused rather than overwriting it.
If undo or redo would recreate a UID already present under another filename, it refuses
before preparing a journal entry, so unrelated writes can continue.

An entry stays pending until a sync confirms the server holds its result; the sync runner then
sets `synced`. A synced entry is pruned `retention_days` (default 30) after confirmation, and
undo keeps working until then. `prepared` and `abandoned` entries are never pruned.

## Locking

`<state>/vdir.lock` is an advisory lock, one per vdir. Writers hold it exclusively for the whole
protocol above. The sync runner holds it shared while vdirsyncer runs, so readers continue and
writers wait; a write during a sync is queued by the daemon, so the TUI never freezes. The lock is
released when the process lets go of it, or by the kernel if the process dies. The daemon itself
is a single instance per vdir, held by `<state>/daemon.lock`; a second daemon for the same
vdir exits, and its client says `another daemon serves <vdir>`.

The sync runner takes its locks in a fixed order: its own run lock (`<state>/sync/run.lock`, so
only one sync runs at a time), then the vdir lock, then the conflicts lock.

One plain file steers the syncs: `<state>/sync/off`, present while automatic sync is off (every
automatic run then skips).

## Reading

The daemon loads the whole vdir into a snapshot and sends it to the TUI, which renders views
from it. The daemon watches the vdir; when files change (an edit, a sync, another program) it
reloads and tells every client, and the TUI fetches the new snapshot. Once a minute the TUI moves
its clock on, so "today" and the date groups stay current. A file that cannot be parsed is listed under `:errors` and left untouched; it does not
stop the rest from loading.

### Explicit sibling order

Reordering and anchored insertion use one family journal entry, submitted through the same
write queue as other edits. They rank the resulting complete sibling sequence with shared
`DisplayOrder` integers, changing only differing ranks. Parent and unchanged sibling bytes
and family membership are durable dependencies, checked on commit, queue drain, recovery
and undo. Queued moves are visible in the overlay and subsequent moves plan against it.
The ordering gate validates family structure and editable JSON, then checks the verified
DisplayOrder operation by declared parent kind. It does not classify recurrence, dates,
alarms or completion state: they are preserved fields. Anchored insertion separately
passes the existing complete Create gate before applying its saved placement.

### Relocation dependencies

List relocation and candidate single-leaf reparenting persist additive `families`
guards alongside the legacy optional `family`. Old journal/queue JSON remains
readable. Each guard records resource/internal parent aliases and known paths;
commit, queue drain, undo/redo and recovery check all guards. Membership rescans
cover other lists too, so a new cross-list standard or JSON-only reference cannot
bypass a loaded family. Exact unchanged parent/sibling bytes remain held.

Candidate reparenting plans new identity and destination rank once, then journals
destination creation before source removal. A partially applied entry reuses that
identity on recovery; it cannot create a second candidate. Both leaf identity guards
recognize the old and new paths, so a retained internal JSON identity does not
conflict with the operation's own source or destination. Destination UID checks
also detect duplicates under other filenames. The separate membership gate keeps
unproven encodings disabled in the shipping matrix.

### Permission report snapshots

Aggregate `tdo check` reports reuse one gate checker for the loaded immutable
snapshot. It indexes resource/internal identity owners, standard/JSON references,
and raw-source metadata ambiguity once, then validates each family against the
same admission rules used for writes. It does not rescan and reparse the entire
collection for every reported task. A reload or post-edit snapshot gets a fresh
checker; no permission or source facts are cached across snapshot changes.
Live commit, queue, recovery and inverse file guards are unchanged.
