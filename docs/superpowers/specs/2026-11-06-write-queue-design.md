# Write queue and batched sync (issue #38)

Date: 2026-11-06 · Status: approved (2026-11-06)

Covers issue #38 (queue writes during a sync), its two comments, and supersedes issue #30
(the after-edit delay). High-level origin: §4.5 of the
[phone web app design](2026-11-05-phone-web-app-design.md). Contracts in the
[main spec](2026-10-31-terminal-tasks-design.md) (§11: compatibility gate, preservation,
conflict, recovery) still bind everything here.

## 1. Why

The user edits several tasks one after another in the TUI. Today every save schedules a
background sync three seconds later, so the sync started by the first edit is often still
holding the vdir when the next edit is saved. The save waits, then fails or asks to stop the
sync (issue #10). The same happens with any sync the user did not start: the systemd/launchd
timer, the startup sync and `tdo sync`.

A longer delay (#30) only makes this rarer. The user wants it gone in every case: **a save
never waits for a sync.**

The user also wants fewer syncs: one edit should not cause a sync on its own. Edits are
batched and sent when enough have piled up or enough time has passed.

## 2. Goals

1. A save in the TUI or `tdo op` never waits for a sync and never asks to stop one. While a
   sync holds the vdir, the edit is queued and the caller returns at once.
2. Queued edits are shown as if applied (with a pending marker) and are applied in order as
   soon as the vdir is free.
3. A queued edit is refused only when it really clashes with a change the sync brought in.
   Non-clashing changes merge (you change the note, the phone completes the task: both
   survive). The same rule applies to ordinary saves made against an outdated view.
4. Syncs caused by edits are batched: a sync starts when the unsynced edit count reaches a
   size, or when a time limit since the first unsynced edit runs out. Both configurable.
5. One switch turns automatic sync off and on (default on), for working offline.
6. Every sync path reports one "sync ended" event; the TUI reloads on it.
7. The TUI status line shows how many edits are pending and whether sync is off.
8. The queue lives in `twydo-core`, so the phone server (#39) can use the same code later.
   The server is its own installation with its own vdir and state directory (phone design
   §5); it runs its own queue and meets the laptop only through CalDAV. Nothing here shares
   a queue between machines or installations.

## 3. Non-goals

- The phone server itself (#39), and any HTTP surface for the queue.
- Changing the timer's interval or the batching values from inside the TUI (the user's
  "later on": on/off now, delay and recurrence later). A sync status view beyond the status
  line (`:sync status` with last sync time and last error) is also later.
- Field-level merging for family operations (projects, checklists, recurring projects):
  they keep today's strict whole-file check (§6.4).
- Merging two edits of the same field. That is a refusal, not a three-way text merge.

## 4. Anti-goals (would be a failure even if it works)

- A queued edit is silently lost: on quit, crash, refusal, or because nobody drained it.
- A queued edit overwrites a change from another device in a field it touches, or silently
  discards a completion, a repeat advancing, or a deletion (main spec, Conflicts).
- A completion of a repeating task is applied twice (once on the phone, once from the
  queue), completing the next occurrence.
- A completed row flips back to open while its edit is still queued.
- Offline work fills the status line with network errors while sync is off.

## 5. Decisions

Marked **(user)** where the user decided, **(my call)** where I did; the builder may refine
my calls with justification.

| # | Decision | Who |
|---|---|---|
| D1 | Full queue, not just a longer delay: saves never wait, whoever started the sync. | user |
| D2 | Batching: sync when the unsynced count reaches `batch_size`, or `max_wait` after the **first** unsynced edit, whichever comes first. Configurable, sensible defaults. | user (rule), my call (counting from first edit, as recommended and accepted) |
| D3 | Defaults: `batch_size = 5`, `max_wait_secs = 60`. | my call |
| D4 | No "every edit pushes the sync back" (#30's idea): the size limit covers bursts. #30 closes as covered. | my call, accepted |
| D5 | `[sync] after_edit` goes away; there is no separate sync-after-edit switch. | user |
| D6 | One automatic-sync switch, default on, persisted in the state directory, honoured by every automatic sync including the timer; explicit `:sync` / `tdo sync` still run. | user (switch), my call (scope and persistence, accepted) |
| D7 | A queued edit is refused only on a clash in the fields it touches (§6.4). | user |
| D8 | A delayed refusal is shown in the status line, naming the task, with the existing wording. | user |
| D9 | Field-level rebasing applies to ordinary saves too: one way edits are applied. | my call, accepted |
| D10 | The #10 "stop the sync and save?" prompt goes away: nothing waits any more. | my call |
| D11 | Quitting does not start a special sync. The batching waiter (§7.2) outlives the TUI and sends unsynced edits within `max_wait`. | my call (replaces the "final sync on quit" I played back; same outcome, one mechanism) |
| D12 | Merging (§6.4 steps 2–4) ships behind a compatibility-matrix row and stays off, with today's strict whole-file check, until the iPhone round passes. The queue itself ships on. | user (accepted my recommendation) |
| D13 | Undo and redo of applied entries are queued too, as their own item kind, with today's strict check. List recovery (`:recover`) still waits, and says "a sync is running; try again" instead of the stop prompt. | my call |
| D14 | The batch count and age cover only unsynced edits made after the last sync attempt began, so entries a sync deliberately left pending (unresolved conflicts) cannot force a sync on every save. | my call |

## 6. The write queue

### 6.1 One write path

`twydo-core` gets one entry point that both the TUI and `tdo op` call
instead of `Store::apply` directly. In order:

1. The gate checks the operation against the caller's current view (the snapshot with the
   queue laid over it, §6.5). A gate refusal comes back immediately, as today.
2. The writer tries the exclusive vdir lock for a short moment (a fraction of a second,
   enough for another `tdo` writer to finish; not long enough to notice). On success it
   first drains the queue (§6.3), then applies this operation through the normal store path
   with rebasing (§6.4). The result is **applied** or **refused**, as today.
3. If the lock is still held, the operation is written to the queue and the result is
   **queued**. After writing the queue file, the writer tries the lock once more and drains
   if it now gets it, so an item queued just as the holder let go is not stranded.

`tdo op` and the TUI keep their own undo stacks in step: an entry is either a journal id
(applied) or a queue id (queued).

### 6.2 On-disk format

- One file per queued operation in `<vdir-state>/queue/<id>.json`, written atomically
  (`fsio::write_atomic`). Several processes can queue at once without sharing a file.
- `<id>` sorts in creation order across processes: creation time, then a per-process
  sequence and pid as tie-breakers. The drain order is the sort order of ids.
- Contents (versioned with a `format` field, like the journal):
  - `id`, `created` (wall clock: also the edit time for batching, §7.1), `origin`
    (`tui` or `op`, plus pid and process start time; the server's own installation can
    add its own origin later).
  - `kind`: either `op`, holding the serialized `edit::Op` (it and its parts need serde),
    or `revert`, holding the journal entry id to undo or redo (D13). A `revert` item's base
    is that entry's `after` bytes and it is always checked strictly, as `u` is today.
  - `base`: for each file the operation reads or writes, the exact bytes the caller saw
    (from its overlaid view, so a second edit of the same task builds on the first queued
    one). For a new task: none. For a family operation: the family guard, as the journal
    has today.
  - `label`: the task title as the user saw it, for messages.
- **Relation to the journal:** the queue holds intent; the journal holds what was written.
  A queued item becomes an ordinary journal entry when it is applied, and that entry records
  the queue id it came from (a new optional `from_queue` field). Nothing about recovery
  changes.

### 6.3 Draining

Whoever holds the exclusive vdir lock with a non-empty queue drains it, oldest id first:

- the sync runner: at the start of each pass (it already takes the exclusive lock for
  recovery) and again after reconciling (step 5), before it lets go;
- any writer that takes the lock (§6.1 step 2);
- the TUI when it receives the "sync ended" event (§8) and the queue is not empty.

For each item: if a journal entry already names it in `from_queue` (a crash after applying,
before removing the file), remove the file and continue. Otherwise rebase and apply it
(§6.4) with the item's recorded `created` time as `now`, then remove the queue file. A
refused item moves to `<vdir-state>/queue/refused/<id>.json` with its reason (§6.6).

A stuck journal (`WriteError::Stuck`) stops the drain; items stay queued and the status line
already reports the stuck entry. Items are never applied out of order to get around it.

### 6.4 Rebasing: when an edit merges and when it is refused

Today a write is refused (`WriteError::Stale`) if any file it touches changed by a single
byte since it was loaded. Under this design (D7, D9), for **single-task operations**:

1. If the file still holds the base bytes: apply as today.
2. Otherwise, parse the base and the current file and compare **only the fields the
   operation touches** (table below). Bookkeeping that every write changes
   (`LAST-MODIFIED`, `DTSTAMP`, `SEQUENCE`, 2Do's modification time in its metadata) is
   not compared.
3. If none of those fields differ: re-run the gate against the current file and re-plan
   the operation on the current file, through the same pure `edit` functions. Every other
   byte keeps the current file's content (the existing preservation rule), so the other
   device's change survives.
4. If the current file already holds exactly what this operation would set (the phone made
   the same change), the edit is satisfied: nothing is written and nothing is reported.
5. Otherwise the edit is refused with the store's reason, naming the task and the field.

| Operation | Fields compared (refuse if changed) |
|---|---|
| `Edit` / `Save` changes | only the fields in its `Change` list: title, notes, due, start, repeat rule and interval |
| `Save` with a move, `Move` | the above, plus the task's list (the file moved or vanished) |
| `Complete` / `Reopen`, non-repeating | status and completion time |
| `Complete` / `Reopen`, repeating | status, completion time, due, start, repeat rule (any advance on another device refuses: never complete the next occurrence) |
| `Change::Kind` (type change) | the whole file (strict, as today): the gate reasons about items |
| `Delete` | the whole file: deleting over someone's edit would discard it |
| `Create` without a parent | nothing: it only fails if the file already exists |

Always refused, whatever the table says:

- the task's file is gone (deleted elsewhere), or now holds a sync conflict;
- the operation touches a family (project, checklist, recurring project, an item of one, or
  a create inside one): today's strict whole-file check and family guard apply unchanged;
- the gate refuses it against the current file (the compatibility gate always has the last
  word).

**Rollout (D12):** steps 2–4 are enabled by a new compatibility-matrix row (for example
`rebase-merge`), `unverified` until the iPhone round in §10 passes. While it is unverified,
every operation uses step 1 or refuses with today's `Stale`, as now; the queue, overlay and
batching work the same either way. Turning the row to verified is the only change needed
afterwards.

The exact mapping from each field to its iCalendar properties and 2Do metadata keys
(the spec's §6 and the compatibility matrix list them) is the builder's call; comparison is
on the parsed values, not raw lines, so two encodings of the same value do not clash.

### 6.5 Showing queued edits

Views lay queued items over the snapshot, in queue order, using the same pure `edit`
functions the store uses to plan (no file is written). Every process overlays every queued
item, whichever process queued it. Rows touched by a queued item carry a pending marker.
When an item is applied, the snapshot itself holds the result and the overlay drops it; when
it is refused, the overlay drops it and the row shows the current file.

### 6.6 Refusals of queued edits

- **TUI:** a refusal of an item that this TUI queued shows in the status line like a normal
  refusal, with the task title: `Not saved: "<title>" changed on another device (notes)`
  (D8). Wording follows today's messages; exact text is the builder's call. Items refused
  while no TUI was running (queued by `tdo op`) show the next time a TUI starts.
- **`tdo op`:** a queued result prints `queued (a sync is running): <message>` and exits 0.
  A `--wait` flag waits until the item is applied or refused and exits with the usual codes,
  for scripts that need the outcome. `tdo op` and `tdo sync` print refusals they find in
  `queue/refused/` that nobody has shown yet, on stderr.
- A refusal counts as shown once a TUI or command has printed it. Refused files are pruned
  with the journal's retention (`retention_days`).

### 6.7 Undo and redo

- `u` on an edit that is still queued removes its queue file. If the file is already gone
  because it was drained, `u` undoes the journal entry that names it in `from_queue`, which
  is today's undo, with today's refusals.
- `Ctrl-r` of such an undo queues or applies the operation again through §6.1.
- Undo of an applied entry keeps today's rules ("undo after a remote edit uses conflict
  handling"), but no longer waits: while a sync holds the vdir it is queued as a `revert`
  item (§6.2), shown in the overlay, and refused at drain time if the files no longer hold
  what the entry wrote. Redo likewise.
- List recovery (`:recover`) is rare and multi-file; it still takes the lock with today's
  wait and, if a sync holds it, says `a sync is running; try again` (D13).

## 7. Batched sync

### 7.1 The rule

- An **unsynced edit** is a journal entry that is applied, not yet confirmed by a sync
  (`Journal::pending()` already lists them), and made after the last sync attempt began
  (`status.last_attempt`, D14). Entries an earlier sync saw and deliberately left pending
  (an unresolved conflict, an uncounted collection) do not count: they wait for the user
  or the next sync, not for batching. Its edit time is the entry's `created`, or for an
  item that came from the queue, the queue item's `created`.
- A sync is **due** when the number of unsynced edits reaches `batch_size`, or when
  `max_wait_secs` have passed since the oldest unsynced edit.
  When the size limit is reached, the sync waits until edits have paused for 3 seconds, so a
  burst of edits is not interrupted by its own sync *(my call, after the final review of
  plan 1)*.
- Queued items do not count until applied; they are applied at the next drain, which the
  running sync does itself, and then fall under the same rule with their original time.

### 7.2 Who starts the sync

After every applied or drained edit, the writer checks the rule (in `twydo-core`):

- If the size is reached: start `tdo sync --background` now (detached, as today).
- Otherwise: make sure one **batch waiter** exists: a detached `tdo sync --background
  --when-due` that sleeps until the oldest unsynced edit's deadline, re-checks the rule (the
  edits may already have been synced by the timer or `:sync`), and runs a sync if it is
  still due. One waiter per vdir, held by a lock file in `<vdir-state>/sync/`; a second
  one exits at once.

The waiter is a separate process, so it works the same for the TUI and `tdo op`, and it
outlives the TUI: quitting with unsynced edits needs no special sync (D11).

**The waiter's lifecycle** *(my call; details are the builder's)*:

1. **Who starts it:** any process that just applied or drained an edit and found the size
   limit not reached: the TUI's writer, `tdo op`, or the sync runner after a drain left
   edits unsynced. It is started the way `spawn_background` starts a sync today: the
   `tdo` executable, detached in its own process group (so closing the terminal does not kill
   it), stdin and stdout closed. Starting it is cheap and unconditional; the lock decides
   whether it stays.
2. **One per vdir:** it takes `<vdir-state>/sync/waiter.lock` without blocking. If another
   waiter holds it, it exits at once.
3. **Loop:**
   - If sync is off, or nothing counts as unsynced (§7.1), it exits.
   - Otherwise it sleeps until the deadline: the oldest unsynced edit's time plus
     `max_wait_secs`, or the runner's backoff time (`status.next_attempt`) if that is later.
     The deadline is wall clock, so after the laptop wakes from sleep it is already past
     and the sync runs at once.
   - On waking it checks again. The timer, `:sync` or a size-triggered sync may have sent
     everything already. If a sync is still due, it runs one in-process (the same
     `sync::run` as `tdo sync --background`), then loops: edits made during that sync get
     their own deadline.
4. **Exit without losing an edit:** before exiting it releases the lock and checks once
   more. If an edit landed in between (whose writer saw the lock held and assumed the
   waiter would cover it), it takes the lock again and loops. If another waiter got the
   lock first, that one covers the edit.
5. **Crash or kill:** the operating system releases the lock. The next write starts a new
   waiter, and the timer (if installed) syncs within a minute regardless.

So it lives about `max_wait_secs` plus the sync's length, and only while unsynced edits
exist. It never runs while sync is off or when nothing is waiting.

The timer (`tdo sync --background` every minute) and the TUI's startup sync stay as they
are: they also pull other devices' changes, which batching does not cover. With the timer
installed, `max_wait` rarely matters; it is the guarantee without one.

`:sync` and `tdo sync` sync now, whatever the count.

### 7.3 Configuration

In `[sync]` (see `docs/config.md`):

| Key | Type | Default | Meaning |
|---|---|---|---|
| `batch_size` | integer ≥ 1 | 5 | Unsynced edits that start a sync at once. `1` gives the old one-sync-per-edit behaviour. |
| `max_wait_secs` | integer ≥ 1 | 60 | Longest an unsynced edit waits before a sync starts. |

`after_edit` is removed (D5). An existing `after_edit` key is accepted and ignored with a
warning, so current configs still load; the builder may decide how long to keep that.

### 7.4 Turning automatic sync off and on

- `:sync off` / `:sync on` in the TUI; `tdo sync off` / `tdo sync on` on the command line.
- Stored as `<vdir-state>/sync/off` (present = off), so it survives restarts and every
  process sees it.
- While off, every automatic sync skips quietly and records nothing as a failure: the
  timer, the startup sync, the batch waiter and the size trigger. A new `Skip::Off` reason
  says so. Explicit `:sync` and `tdo sync` still run once.
- The queue keeps working while off (a manual sync can still hold the vdir).
- Turning it on with unsynced edits starts a sync at once.
- While off, the status line always says so with the unsynced count (§8), so the user does
  not forget their changes are only local.

## 8. The "sync ended" event and the status line

**Event (from the #43 comment):** every runner invocation that did anything (ran
vdirsyncer, drained the queue, or was stopped) ends by atomically writing
`<vdir-state>/sync/ended` with a run id and time. Skipped runs (`Skip::*`) do not write it.
The TUI watches that file and, on any change, reloads the snapshot, refreshes the overlay
and drains the queue if it is not empty. This replaces the manual-`:sync`-only reload from
PR #43 and covers the startup sync, the waiter, the timer and systemd runs. Whether it is
a separate file or a field in `status.json` is the builder's call; it must not depend on
the file watcher seeing vdir changes.

**Status line:** the existing sync segment gains:

- `N pending`: unsynced plus queued edits, while N > 0;
- `sync off` while the switch is off (always visible, with the count);
- the pending marker on rows touched by queued items (§6.5).

Exact symbols and colours are the builder's call, consistent with the current theme.

## 9. Changes to existing behaviour

- **Issue #10 prompt removed:** `sync running for … — stop it and save? (y/n)` no longer
  appears, because saves no longer wait. `request_stop` stays (the runner's timeout and
  `tdo sync` still use the stop machinery); exposing `:sync stop` for a stuck sync is the
  builder's call.
- **`Store::set_lock_wait` and `WriteError::Busy`** remain for callers outside the new path,
  but the TUI and `tdo op` no longer see `Busy`.
- **`WriteError::Stale`** becomes rarer (§6.4) and names the clashing field.
- **`SYNC_DELAY` and `Writer::sync_soon` / `sync_if_quiet`** are replaced by §7.
- **Issue #30** closes as covered by §7.

## 10. Tests

At least:

- Queue: a write while another process holds the shared vdir lock returns queued without
  waiting; the item is applied by the next lock holder, in id order across two processes.
- The lost-wakeup case: the lock is released between the failed try and the queue write;
  the writer's re-check drains it.
- Crash between applying a queued item and removing its file: the next drain removes it
  without applying it twice (`from_queue`).
- Rebasing, one test per table row in §6.4: merge (note vs completion, title vs due),
  refusal (same field), satisfied (same change on both sides), repeating completion after
  a remote advance (refused), delete over an edit (refused), family operation (strict).
- The gate refuses on the current file at drain time even though it allowed the queued op.
- Overlay: a queued completion keeps the row completed through reloads until applied; a
  refused one returns to the current file.
- Undo of a queued item removes it; undo after it drained undoes the journal entry.
- Batching: size trigger, time trigger from the oldest edit, one waiter per vdir, the
  waiter re-checks and exits when the timer already synced.
- Waiter exit race: an edit lands after the waiter's last check but before it releases
  the lock; the waiter's re-check after release picks it up. A waiter wakes past its
  deadline (laptop sleep) and syncs at once; it honours backoff.
- Sync off: timer, startup, waiter and size trigger skip with `Skip::Off`; `tdo sync` still
  runs; turning on with unsynced edits syncs at once.
- "Sync ended" is written by every non-skipped run kind, and the TUI reloads on it.
- `tdo op` prints queued and exits 0; `--wait` reports the final outcome.

Device rounds: rebasing against a phone edit is the first real use of merging two devices'
changes into one write. Before relying on it, verify the merge rows of §6.4 against the
iPhone with the user's approval and a bounded checklist (AGENTS.md; use
`tools/seed_fixture.py` on the synthetic test lists).

## 11. Documentation to update with the change

`docs/sync.md` (triggers, batching, off switch, queue), `docs/config.md` (§7.3),
`docs/tui.md` (status line, pending marker, `:sync on/off`, the removed stop prompt),
`docs/architecture.md` (queue, drain, rebasing, the event), `docs/tasks.md` if refusal
wording changes, and `AGENTS.md`'s progress note. `python3 tools/check_links.py` must pass.

## 12. Left to the builder

Module layout in `twydo-core` (a `queue` module next to `store` is the obvious place), id
format details, the serde shape of `Op`, the lock-try duration in §6.1, the exact field to
property mapping in §6.4, message wording within today's style, status-line symbols, how
long the ignored `after_edit` key is accepted, whether the event is its own file, whether
`:sync stop` exists, and splitting the work into plans (a natural split: queue and
rebasing first, then batching and the switch, then the event and status line).
