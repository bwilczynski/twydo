# Architecture

How Twydo is put together, for contributors and for anyone who wants to know what happens to
their files. For what a task looks like on disk, see [data-model.md](data-model.md).

## The pieces

```
 iPhone (2Do) ──CalDAV──▶ CalDAV server ◀──CalDAV── vdirsyncer ──files──▶ vdir ◀── tdo
                                                         ▲                         │
                                                         └──── tdo sync runs it ───┘
```

- **The vdir** is the source of truth on your machine: a folder with one subfolder per list
  and one `.ics` file per task, kept in step with the CalDAV server by
  [vdirsyncer](https://vdirsyncer.pimutils.org/). Twydo never talks CalDAV itself.
- **`tdo`** reads the vdir, shows it in the TUI and writes changes back as files. `tdo sync`
  runs vdirsyncer for you, with recovery before and conflict handling after
  ([sync.md](sync.md)).
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

## Crates

The repository is a Cargo workspace with two crates.

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

**`tdo`**, the binary: the CLI subcommands (`cli/`), the TUI's state and keys (`app.rs`), the
edit card's state (`form.rs`), the terminal loop with live reload (`term.rs`), and rendering (`ui/`).

## The write path

Every write, from a TUI key, `tdo op` or a conflict resolution, goes the same way:

1. **Gate.** `gate::check` decides whether the operation is allowed for this task. It looks up
   the task's case (one-time, monthly, yearly, checklist, project, …), the matrix rows for that
   case and operation, and the verdicts for its repeat rule class. In the default `verified`
   mode, anything the matrix has not marked as passing is refused with a reason, before
   anything is written.
2. **Edit.** The `edit` functions produce the new file contents. Only the properties the
   operation is about change, plus `DTSTAMP`, `LAST-MODIFIED` and their counterparts in 2Do's
   metadata; every other line keeps its bytes.
3. **Store.** Under the exclusive vdir lock, the store checks that every file it is about to
   write still holds the bytes the caller loaded (otherwise: "changed on disk; reload and try
   again"), records the operation in the journal, applies it, and marks it applied. Files the
   operation relies on but does not write are checked the same way: the parent of a new item,
   and the family members it leaves as they are. An operation whose result depends on a task's
   items (a family operation, a checklist's date change, a change of type) also rescans the
   list and is refused if an item arrived after the load.

An operation that touches a family (a project or checklist and its items) writes the parent and
every affected child in one journal entry, so it is applied completely or not at all.

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
  Recurring-checklist item operations retain the parent bytes (and deletion also
  retains sibling bytes) here. Apply, recovery, undo and redo require them to remain
  unchanged. They are never written or treated as sync mutations. Older entries
  without this field have no durable read dependencies.
- `family` (optional): the collection, parent UID and loaded family paths for an item
  creation or an individual recurring-checklist item deletion. Under the vdir lock, a rescan rejects newly joined
  members before applying, undoing, redoing or recovering the deletion. Byte dependencies
  guard the original members; an operation's own created/deleted path is included and may
  be present/absent for idempotent recovery. Item creation records sibling byte dependencies
  because its append position depends on the complete loaded sibling set.
  This guard is retained by inverse entries and is absent from older journal entries.

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

Recovery runs at the start of every command that reads the vdir (`tdo` / `tdo tui`,
`export-json`, `check`, `op`), before every sync, and before every write:

- with no journal directory, it does nothing and creates nothing;
- otherwise, under the exclusive lock, every `prepared` entry is rolled forward and marked
  `applied`; `applied` and `abandoned` entries are left alone;
- a `prepared` entry that conflicts (a file holds neither `before` nor `after`, for example
  because a sync brought in another device's change), has an invalid path, or cannot be parsed
  stays as it is and is reported as **stuck**, with the reason. Nothing is written for it.
  `tdo op journal` lists it; `tdo op abandon <id>` gives up on it;
- an I/O error stops recovery; the entry stays `prepared` and is retried next time.

Recovery is idempotent: a second run finds nothing to apply and changes nothing.

At the start of a command, recovery does not wait for a running sync: when a sync holds the vdir
lock, it is skipped, so `tdo` opens at once. Nothing is lost by that. The sync recovered before
vdirsyncer started, and the next write recovers under the lock before it changes anything.

### Undo, redo and retention

`u` and `Ctrl-r` in the TUI (and `tdo op undo <id>`) write a new journal entry that reverses an
earlier one, under the same "file must still hold the expected bytes" rule: an undo after a
sync brought in someone else's change to the same task is refused rather than overwriting it.

An entry stays pending until a sync confirms the server holds its result; the sync runner then
sets `synced`. A synced entry is pruned `retention_days` (default 30) after confirmation, and
undo keeps working until then. `prepared` and `abandoned` entries are never pruned.

## Locking

`<state>/vdir.lock` is an advisory lock, one per vdir. Writers hold it exclusively for the whole
protocol above. The sync runner holds it shared while vdirsyncer runs, so readers continue and
writers wait; the TUI does not wait for it: a write during a sync is refused with "a sync is
running; try again in a moment", so the screen never freezes. The lock is released when the
process lets go of it, or by the kernel if the process dies.

The sync runner takes its locks in a fixed order: its own run lock (`<state>/sync/run.lock`, so
only one sync runs at a time), then the vdir lock, then the conflicts lock.

## Reading

The TUI loads the whole vdir into a snapshot, renders views from it, and reloads when files in
the vdir change (for example after a sync) and when a manual `:sync` finishes, without waiting
for the watcher. Once a minute it moves its clock on, so "today" and the date groups stay
current. A file that cannot be parsed is listed under `:errors` and left untouched; it does not
stop the rest from loading.
