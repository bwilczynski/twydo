# Operation journal

Every write Twydo makes to the vdir goes through the operation journal
(`twydo_core::store::journal`), so a crash at any point leaves the data either
untouched or recoverable to the complete result of the operation, exactly once.
This is the mechanism behind the spec's §6 Recovery contract.

## Directory layout

```
<state_dir>/                  e.g. $XDG_STATE_HOME/twydo/
  vdirs/
    <id>/                     one per vdir: <state> below
      vdir                    identity file: the canonical vdir path and a newline
      vdir.lock               advisory lock file (see "The lock")
      journal/
        20261101T120000Z-3fa9c2d1.json   one file per operation, named <id>.json
        .20261101T120000Z-3fa9c2d1.json.tdo-tmp   transient, only during a write
```

Journal write paths are relative to the vdir, so a journal belongs to exactly one
vdir: replaying it into another one with the same collection names would write
there. Each vdir therefore has its own state directory, `<state>` in the rest of
this document. `<id>` is the FNV-1a 64 hash of the canonical vdir path (symlinks
and `.` resolved) as 16 lowercase hex digits (`twydo_core::store::vdir_state_dir`).

- Writers (`Store::open_with`, hence every `tdo op` command) use
  `bind_vdir_state`: it creates `<state>` and writes the identity file if it is
  missing.
- Readers (startup recovery, the shared lock while loading, `tdo op journal`)
  use `existing_vdir_state`: it creates nothing and returns nothing when
  `<state>` does not exist, so a vdir nothing has written to has no state.
- Both refuse (`InvalidData`, naming both paths) when the identity file names a
  different path than the vdir being opened — only a hash collision or manual
  tampering does that. Its journal is never replayed; move that directory away
  to continue.
- A moved or renamed vdir hashes to a new `<id>`, so it starts with an empty
  state directory. Its old journal, including any unfinished `prepared`
  entries, is left behind under the old `<id>` and is not recovered. Before
  moving a vdir, finish or abandon its unfinished operations (`tdo op journal`,
  `tdo op abandon`); otherwise move the entries of the old `vdirs/<id>/journal/`
  into the new one by hand.

Journals from before this layout, at `<state_dir>/journal/`, are ignored: they
are not tied to a vdir, so they cannot be replayed safely.

The operation id is `<YYYYMMDDTHHMMSSZ>-<8 hex>`: the UTC creation time and 8
random hex digits (`new_op_id`). Ids sort chronologically, and recovery and
`Journal::list` process entries in id order.

All files, in the journal and in the vdir, are written with `write_atomic`: the
bytes go to `.<name>.tdo-tmp` in the same directory, the temp file is fsynced,
renamed over the target, and the directory is fsynced. Removals use
`remove_durable` (unlink, then fsync the directory; a missing file is not an
error).

## Entry format (format 1)

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
- `uids`: the task UIDs the operation targets.
- `stage`: `prepared`, `applied` or `abandoned`.
- `inverse_of`: for `undo`/`redo`, the id of the operation being reversed.
- `writes`: in application order. `path` is relative to the vdir
  (`<collection>/<file>.ics`); absolute paths and `..` components are rejected.
  `before`/`after` are the **full file contents**; `null` means the file is
  absent (so `before: null` creates a file and `after: null` deletes one).

Writes are ordered so that a history copy is created before its live task
advances: if the operation stops halfway, there is never a live task that has
moved on without its completed occurrence on disk.

## Protocol

A writer performs one operation as:

1. take the exclusive lock (`VdirLock::exclusive`);
2. `Journal::prepare` — the entry is durably on disk with stage `prepared`
   before the vdir is touched;
3. `apply_entry` — the writes, in order;
4. `Journal::set_stage(id, Applied)`;
5. release the lock (drop the `VdirLock`).

`apply_entry` handles each write by comparing the file's current bytes:

| current content | action                                             |
|-----------------|----------------------------------------------------|
| equals `after`  | skip (already applied)                             |
| equals `before` | write `after` atomically, or remove if `after` is null |
| anything else   | `Conflict { path }`                                 |

Every write is checked before any is made, so a conflict leaves the vdir
untouched. Because applied writes are skipped, applying an entry twice has the
same effect as applying it once.

## Recovery

`recover(vdir, state)` runs at the start of every entry point that reads the
vdir (`tui`, `export-json`, `check`, `op`):

- If `<state>/journal` does not exist it does nothing and creates nothing.
- Otherwise it takes the exclusive lock and runs `recover_locked`.

`recover_locked(vdir, journal)` is for callers that already hold the exclusive
lock (taking it a second time on another descriptor would deadlock, even in the
same process). For every entry in id order:

- `applied` and `abandoned` entries are left alone;
- a `prepared` entry is rolled forward with `apply_entry`, then marked
  `applied`, and its id is listed in `RecoveryReport::resumed`;
- a `prepared` entry that conflicts (a file holds neither `before` nor `after`,
  e.g. a sync brought in another device's change), has an invalid path, or whose
  JSON cannot be parsed stays as it is and is listed in `RecoveryReport::stuck`
  with the reason. Nothing is written for it; resolving it is up to the user.
- An I/O error aborts recovery with the error; the entry stays `prepared` and is
  retried next time.

Recovery is idempotent: a second run finds no `prepared` entry it can apply and
changes nothing.

## The lock

`<state>/vdir.lock` is an advisory `flock`-style lock (`std::fs::File::lock`),
one per vdir.
Writers hold it exclusively for the whole protocol above; readers that must not
see a half-applied operation — the notifier and the sync runner — hold it
shared (`VdirLock::shared`). Taking either lock creates the state directory and
the lock file if they are missing. The lock is released when the `VdirLock` is
dropped (and by the kernel if the process dies).

## Retention

Applied entries are kept: they are the record undo/redo and conflict diagnosis
work from. Pruning them once a sync has confirmed the server holds the result is
Plan 3's job; until then the journal only grows.
