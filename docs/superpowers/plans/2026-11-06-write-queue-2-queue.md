# Write queue, plan 2: the queue (strict checks) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A save in the TUI or `tdo op` never waits for a sync: while the vdir lock is held the edit is queued, shown as if applied, and applied in order by the next lock holder, with today's strict whole-file checks.

**Architecture:** A new `twydo_core::store::queue` module owns the on-disk queue (`<vdir-state>/queue/`). `Store::apply` is split into *prepare* (gate + plan, no lock) and *commit* (checks + journal, under the lock); a new `Store::submit` either commits now or writes the prepared entry to the queue. Draining re-runs the gate on a fresh snapshot and commits each item under the lock (writer, sync runner, TUI). Views lay queued writes over the loaded files. Field-level rebasing (spec §6.4 steps 2–4) is plan 3; here every check is strict.

**Tech Stack:** Rust workspace (`twydo-core`, `tdo`), serde/serde_json, chrono, ratatui; `cargo test`, temp dirs, the fake vdirsyncer `Fx` in `crates/twydo-core/tests/sync_run.rs`.

**Spec:** `docs/superpowers/specs/2026-11-06-write-queue-design.md` §6 (except §6.4 steps 2–4), §7.1 (queued edit time), §8 (status line, pending marker), §9, §10, D7–D13. Plan 1 (`2026-11-06-write-queue-1-batched-sync.md`) is merged: `sync::{ended, switch, batch, waiter}` exist.

## Decisions this plan makes (builder's calls in spec §12)

- **The queue stores the prepared entry, not only the op.** An `op` item holds the `Op` *and* the journal entry planned against the caller's overlaid view (its `writes[].before` and `holds` are the spec's `base`; its `family` is the family guard). Draining commits those exact writes after the strict checks, so a queued `Create` keeps the UID the view showed, and the overlay is exactly what will be written. Plan 3 re-plans only when a base differs.
- **Claiming:** a drainer renames `queue/<id>.json` → `queue/<id>.claim` before committing; cancel (undo) removes `queue/<id>.json`. Whichever filesystem call wins decides; no lock is needed for cancel.
- Lock try in `submit`: `TRY_LOCK = 250 ms` (retry every 25 ms, as `LOCK_RETRY`).
- Queue id: `<%Y%m%dT%H%M%S%6fZ>-<seq:08>-<pid:010>` (fixed widths, so string order = creation order; passes the journal's id charset).
- Pending marker: the row's title gets a muted ` (queued)` suffix. Status count `N pending` = unsynced journal entries + queued items.
- Refusal wording for a queued item: `Not saved: "<label>" changed on another device` for `Stale`; any other refusal: `Not saved: "<label>": <error>`.
- `:sync stop` is not added.

## Global Constraints

- One write path for the TUI and `tdo op`: `Store::submit`; they never see `WriteError::Busy` from it (spec §6.1, §9).
- Drain order = id sort order; a stuck journal (`WriteError::Stuck`) stops the drain, items stay queued (§6.3).
- A queued item is applied at most once: an item whose id is named by a journal entry's `from_queue` is removed, not applied (§6.3).
- The gate re-checks every item on the current files at drain time (§6.4 "always refused").
- Queued items use `created` as `now` for planning; the journal entry's `created` is the drain time; batching ages a drained entry by `from_queue.queued` (§7.1, D14).
- `:recover` still waits for the lock; when a sync holds it the message is `a sync is running; try again` (D13).
- The #10 prompt `sync running for … — stop it and save? (y/n)` is removed; `request_stop` stays (D10, §9).
- No personal data in tests or docs (AGENTS.md). Behaviour changes update `docs/` in the same task; `python3 tools/check_links.py` passes.
- Commit messages: conventional commits, no agent attribution.

## Review Focus

1. **Two edits of the same task while a sync runs** (edit title, then complete): the second must build on the first, both apply in order, and the row shows both meanwhile — test in Task 3 (`second_queued_edit_builds_on_the_first`) and Task 4.
2. **A new task created while queued, then edited before the drain:** the edit targets the UID the view showed — test in Task 3 (`queued_create_keeps_its_uid`).
3. **Undo pressed while the sync is draining that very item:** neither double-applied nor silently kept — test in Task 5 (`undo_after_claim_reverts_the_entry`).
4. **The TUI is quit with items still queued:** nothing lost; the running sync drains them and a later sync sends them — test in Task 6 (`runner_drains_and_reruns_to_send`).
5. **Phone changed a task that has a queued edit:** strict refusal, row returns to the file, the message names the task — test in Task 3 and Task 7.

---

### Task 1: Queue storage

**Files:**
- Create: `crates/twydo-core/src/store/queue.rs`
- Modify: `crates/twydo-core/src/store/mod.rs` (`pub mod queue;`), `crates/twydo-core/src/edit/mod.rs` (serde on `Due`, `Change`, `NewTask`, `Op`), `crates/twydo-core/src/repeatinput.rs` (`RepeatSpec`), `crates/twydo-core/src/model/task.rs` (`TaskKind`) and whatever `RepeatSpec` contains, `crates/twydo-core/src/store/journal.rs` (`Entry.from_queue`), `AGENTS.md` (progress note: "plan 1 merged (PR #50); plan 2 in progress")
- Test: `crates/twydo-core/tests/queue.rs` (new)

**Interfaces:**
- Produces (all in `store::queue`, all `Serialize + Deserialize`, JSON with a `format: u32 = 1` field on files):
  - `pub struct Origin { pub tool: String /* "tui" | "op" */, pub pid: u32, pub started: DateTime<Utc> /* process start, a OnceLock set on first use */ }`; `pub fn origin(tool: &str) -> Origin`.
  - `pub struct Prepared { pub entry: journal::Entry, pub holds: Vec<(String, String)>, pub family: Option<FamilyGuard> }` — `entry.id` is replaced at commit.
  - `pub enum Kind { Op { op: Op, prepared: Prepared }, Revert { entry: String, writes: Vec<Write> } }` (`#[serde(tag = "kind", rename_all = "lowercase")]`); `writes` of a revert are the swapped writes, for the overlay.
  - `pub struct Item { pub format: u32, pub id: String, pub created: DateTime<Utc>, pub origin: Origin, pub label: String, #[serde(flatten)] pub kind: Kind }` and `impl Item { pub fn writes(&self) -> &[Write] }`.
  - `pub fn new_id(now: DateTime<Utc>) -> String` (format above; process-wide `AtomicU64` seq).
  - `pub struct Queue { dir: PathBuf }` with: `open(state: &Path) -> io::Result<Queue>` (creates `queue/` and `queue/refused/`); `existing(state: &Path) -> Option<Queue>` (creates nothing); `push(&self, item: &Item) -> io::Result<()>` (`fsio::write_atomic`); `list(&self) -> io::Result<Vec<Item>>` (both `.json` and `.claim`, sorted by id; unreadable files skipped with no panic); `len(&self) -> io::Result<usize>`; `claim(&self, id) -> io::Result<bool>` (rename `.json`→`.claim`; `false` on NotFound; `true` if already a `.claim`); `cancel(&self, id) -> io::Result<bool>` (remove `.json`; `false` on NotFound); `done(&self, id) -> io::Result<()>` (remove `.claim` durably); `refuse(&self, item: &Item, reason: &str, at) -> io::Result<()>` (writes `refused/<id>.json` = `Refusal`, then `done`); `unshown(&self) -> io::Result<Vec<Refusal>>`; `mark_shown(&self, id) -> io::Result<()>`; `prune(&self, before: DateTime<Utc>) -> io::Result<usize>` (refused files with `at < before`).
  - `pub struct Refusal { pub format: u32, pub item: Item, pub reason: String, pub at: DateTime<Utc>, pub shown: bool }`.
  - `journal::Entry.from_queue: Option<FromQueue>` with `pub struct FromQueue { pub id: String, pub queued: DateTime<Utc> }` (`serde(default, skip_serializing_if = "Option::is_none")`); every existing `Entry { … }` literal gets `from_queue: None`; inverses (`revert`) keep `None`.
  - `Journal::by_queue_id(&self, id: &str) -> io::Result<Option<Entry>>`.

- [ ] **Step 1: Write the failing tests** in `tests/queue.rs`:
  - `ids_sort_in_creation_order`: 1000 `new_id(now)` with the same `now` are strictly increasing as strings; an id from `now + 1µs` sorts after all of them.
  - `item_roundtrips`: an `Op::Save` with a `Change::Repeat(Some(..))` and an `Op::Create(NewTask{ parent: Some(..), .. })` item survive `push` + `list` equal.
  - `old_journal_entries_load_without_from_queue`: an entry JSON without the field parses; `from_queue` is `None`; a written entry with `None` has no `from_queue` key.
  - `claim_then_cancel_loses` / `cancel_then_claim_loses`: exactly one of `claim`/`cancel` returns `true`.
  - `list_includes_claimed_items`, `refuse_moves_to_refused_and_is_unshown_until_marked`, `prune_drops_old_refusals`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test queue` — expect FAIL (module missing).
- [ ] **Step 3: Implement** the module, serde derives, `from_queue`, `by_queue_id`, and the AGENTS.md note.
- [ ] **Step 4: Run** `cargo test -p twydo-core` — expect PASS.
- [ ] **Step 5: Commit** `feat(store): on-disk write queue`

### Task 2: Prepare/commit split and `Store::submit`

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs`
- Test: `crates/twydo-core/tests/store_queue.rs` (new; reuse `tests/common` fixtures)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `Store` keeps `cfg: Config` (cloned in `open_with`) and `pub(crate) fn for_drain(cfg: &Config) -> Result<Store, WriteError>` (no recovery; embedded matrix) for callers that already hold the lock.
  - `fn prepare(&self, snap: &Snapshot, op: &Op, now) -> Result<Prepared, WriteError>` — today's `apply` up to building the entry (gate, plan, holds, family). `fn commit_locked(&mut self, p: Prepared, from_queue: Option<FromQueue>, now) -> Result<String, WriteError>` — `check_not_stuck`, holds/family checks, `run`. `apply` = prepare + `lock()` + commit (unchanged behaviour; all existing store tests pass untouched).
  - `pub enum Submitted { Applied(String /* journal id */), Queued(Item) }`.
  - `pub fn submit(&mut self, view: &Snapshot, op: &Op, label: &str, tool: &str, now) -> Result<Submitted, WriteError>` — spec §6.1: prepare; try the lock for `TRY_LOCK`; on success `drain_locked` then commit → `Applied`. Otherwise push an `Item`, then `VdirLock::try_exclusive` once: if acquired, `drain_locked` and return `Applied(id)` / the refusal error for our own item; else `Queued(item)`. Pushes onto the session undo stack (Task 5 changes its type; until then push only `Applied` ids).
  - `pub struct Drained { pub item: Item, pub outcome: DrainOutcome }`, `pub enum DrainOutcome { Applied(String), AlreadyApplied(String), Refused(String) }`.
  - `pub(crate) fn drain_locked(&mut self, _held: &VdirLock, now) -> Result<Vec<Drained>, WriteError>` — `vdir::load` once, then for each `Queue::list` item: `by_queue_id` hit → `done`, `AlreadyApplied`; `claim` false → skip (cancelled); `Op` → `gate::check` on the current snapshot with `today` from `item.created`, then `commit_locked(prepared, Some(FromQueue{id, queued: created}), now)`; `Revert` → `revert_locked(entry, from_queue)`. `Ok` → `done`, and lay the writes over the loaded report (Task 3's `overlay_writes`; until Task 3, reload with `vdir::load`). `Stuck` → stop, leave the claimed item as `.claim`, return what was drained plus the error only if nothing drained. Any other error → `refuse(item, reason)`, `Refused(reason)`.
  - `pub fn drain(&mut self, now) -> Result<Vec<Drained>, WriteError>` — try the lock for `TRY_LOCK`; `Busy` if held; for the TUI's "sync ended" path.
  - `pub fn drain_with_lock(cfg: &Config, held: &VdirLock, now) -> Result<Vec<Drained>, WriteError>` (`store::queue` free fn over `Store::for_drain`) for the runner.

- [ ] **Step 1: Write the failing tests** (hold the lock with `VdirLock::exclusive(state)` in the test to play the sync):
  - `submit_with_free_lock_applies`: `Applied`, file written, queue empty.
  - `submit_while_locked_queues_without_waiting`: elapsed < 1 s, `Queued`, file unchanged, one item in the queue.
  - `next_lock_holder_drains_in_id_order`: two `Store`s (two "processes") each queue one edit of different tasks while locked; release; a third `submit` applies both first, in id order (journal `from_queue` ids ascend), then its own.
  - `lost_wakeup_is_drained_by_the_recheck`: inject the lock release between the failed try and the push (expose `#[doc(hidden)] pub fn submit_with(…, after_push: &mut dyn FnMut())`, `submit` passes a no-op); the result is `Applied`.
  - `crash_after_apply_is_not_applied_twice`: commit an item, then restore its queue file as `.claim` (simulating a crash before `done`); drain → `AlreadyApplied`, file content unchanged, one journal entry.
  - `drain_regates_on_current_files`: queue a `Complete` allowed by the view; before draining, rewrite the file with an unsupported repeat rule (fixture in `tests/data`); drain → `Refused`, item in `refused/`, file untouched.
  - `stale_queued_edit_is_refused` (Review Focus 5): queue a title edit; change the file's notes on disk; drain → `Refused` with a `Stale` reason.
  - `stuck_journal_stops_the_drain`: a Prepared entry that cannot finish; drain returns `Stuck`; item still listed.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test store_queue` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p twydo-core` — expect PASS (existing `store.rs`, `store_family.rs` unchanged).
- [ ] **Step 5: Commit** `feat(store): one write path that queues while the vdir is busy`

### Task 3: Overlay and the pending marker (core)

**Files:**
- Modify: `crates/twydo-core/src/vdir.rs` (`LoadReport::overlay_writes`), `crates/twydo-core/src/views.rs` (`Snapshot.pending`), `crates/twydo-core/src/store/queue.rs` (`view`), `crates/twydo-core/src/sync/mod.rs` (`SyncSummary.queued`)
- Test: `crates/twydo-core/tests/store_queue.rs`, `crates/twydo-core/tests/vdir.rs`

**Interfaces:**
- Produces:
  - `impl LoadReport { pub fn overlay_writes(&mut self, vdir: &Path, writes: &[Write]) }` — for each write: drop tasks whose `path` is `vdir/<path>` and that path's error; `after: Some` → parse, push tasks (collection = first path component), set `sources`; `None` → remove the source. Then re-run `flag_duplicate_uids`.
  - `pub fn view(report: LoadReport, cfg: Config) -> Snapshot` in `store::queue` — reads the queue (no state or read error → plain `Snapshot::new`); in id order, lays an item over only if every `before`/hold it names still matches the overlaid sources (strict: otherwise it will be refused, so the row shows the file); marks the item's `entry.uids` (or the reverted entry's uids) pending.
  - `Snapshot.pending: HashSet<String>` (private) with `pub fn is_pending(&self, idx: usize) -> bool` and `pub fn with_pending(self, uids: HashSet<String>) -> Snapshot`.
  - `SyncSummary.queued: usize` (queue length; 0 without state).

- [ ] **Step 1: Write the failing tests:**
  - `vdir.rs`: `overlay_replaces_creates_and_removes` — one write each kind; tasks and sources match a fresh `load` of the same files written to disk.
  - `queued_completion_stays_completed_until_applied`: queue a `Complete` while locked; `view` twice (two reloads) → task `Completed` and `is_pending`; drain; `view` → `Completed`, not pending.
  - `refused_item_returns_to_the_file`: queue a title edit, change the file on disk, `view` → shows the file's title, not pending.
  - `second_queued_edit_builds_on_the_first` (Review Focus 1): while locked, submit a title edit against `view`, then `Complete` against the next `view`; drain → both applied, second's `before` == first's `after`.
  - `queued_create_keeps_its_uid` (Review Focus 2): queue a `Create`; the view shows it with UID U; submit `Edit{uid: U}` against that view; drain → one file, edited, UID U.
  - `summary_counts_queued_items`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test store_queue --test vdir` — expect FAIL.
- [ ] **Step 3: Implement**, and make `drain_locked` use `overlay_writes` instead of reloading.
- [ ] **Step 4: Run** `cargo test -p twydo-core` — expect PASS.
- [ ] **Step 5: Commit** `feat(views): show queued edits over the loaded files`

### Task 4: Batching ages a drained edit by its queue time

**Files:**
- Modify: `crates/twydo-core/src/sync/batch.rs` (`decide`, `due`)
- Test: `crates/twydo-core/tests/sync_batch.rs`

**Interfaces:**
- Produces: `decide(edits: &[(DateTime<Utc>, DateTime<Utc>)], …)` — each pair is `(written, edited)`: the D14 filter uses `written >= last_attempt`, the age and the quiet period use `edited`. `due` passes `(e.created, e.from_queue.map_or(e.created, |q| q.queued))`.

- [ ] **Step 1: Update existing `decide` tests** to pairs `(t, t)`; add `drained_edit_counts_from_its_queue_time`: written `now-1s`, edited `now-2m`, `last_attempt = now-30s` → `Now` (counted, and past its deadline).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_batch --test sync_waiter` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same — expect PASS.
- [ ] **Step 5: Commit** `fix(sync): age drained edits from when they were queued`

### Task 5: Undo and redo of queued items

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs`
- Test: `crates/twydo-core/tests/store_queue.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `pub enum Step { Applied(String), Queued(Item), Cancelled(Item) }`; `Store.done` and `Store.undone` become `Vec<Step>`.
  - `undo_last(now) -> Result<Submitted, WriteError>` pops `done`, `redo_last` pops `undone`; both "revert" the step and push the result on the other stack:
    - `Applied(id)` → `revert_or_queue(id)`: try the lock for `TRY_LOCK`; held → push a `Revert { entry: id, writes }` item (overlay writes = the swapped writes; `revert`'s checks run at drain) and return `Queued`; else today's `revert`.
    - `Queued(item)` → `cancel(id)` true → step `Cancelled(item)`, result `Submitted::Cancelled`. `cancel` false → `by_queue_id(id)` → `revert_or_queue(entry.id)`; not found (a drainer is mid-commit) → `Err(WriteError::Busy)`, step stays on its stack.
    - `Cancelled(item)` → push the same item back (same id) and try the lock once, as `submit` does → `Queued` or `Applied`.
  - `Submitted` gains `Cancelled`.
  - `undo(id, now)` (by id, used by `tdo op undo`) keeps its behaviour but goes through `revert_or_queue`.

- [ ] **Step 1: Write the failing tests:** `undo_of_a_queued_item_removes_it` (queue empty, file untouched, view no longer pending); `redo_requeues_it` (then drain applies it); `undo_after_drain_reverts_the_entry` (drain first; undo → `Applied`, file back to the original); `undo_after_claim_reverts_the_entry` (Review Focus 3: `claim` the item by hand, as a drainer mid-commit would; undo → `Busy`, the step is still on `done`; then drain it; undo again → reverts the entry, file back to the original); `undo_while_locked_queues_a_revert` (applied edit; lock held; undo → `Queued`; view shows the original; drain → file restored, journal entry `inverse_of` set and `from_queue` set); `queued_revert_is_refused_if_the_file_changed`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test store_queue --test store` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p twydo-core` — expect PASS.
- [ ] **Step 5: Commit** `feat(store): undo and redo of queued edits`

### Task 6: The sync runner drains

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs` (`recover_and_park`, step 5, `RunReport.drained`), the journal-prune site in `crates/twydo-core/src/sync/reconcile.rs` (also `Queue::prune` with the same cutoff), `crates/tdo/src/cli/sync.rs` (after a non-skipped run with applied drains, `after_write` + `spawn`; print unshown refusals on stderr and mark them shown)
- Test: `crates/twydo-core/tests/sync_run.rs`, `crates/tdo/tests/cli.rs`

**Interfaces:**
- Consumes: `queue::drain_with_lock` (Task 2).
- Produces: `RunReport.drained: Vec<Drained>`. Step 3 drains after recovery when nothing is stuck, under the same exclusive lock. Step 5 drains after `reconcile`, under `_excl`; if it applied anything and this is not the last pass, it creates `sync/requested` so the loop runs another pass and sends them. A drain error is a warning in `report.warnings`, never a failed run.

- [ ] **Step 1: Write the failing tests:**
  - `runner_drains_before_syncing`: queue an edit (hold the lock in the test, `submit`, release); background run → the fake vdirsyncer sees the edited file (fake copies the file to a marker); `report.drained` has it `Applied`; entry confirmed.
  - `runner_drains_and_reruns_to_send` (Review Focus 4): the fake's first call queues an edit through a second `Store` (it is blocked by the shared lock, so `submit` queues); after the run the edit is applied and confirmed by the second pass (fake called twice).
  - `drain_refusal_is_a_warning_not_a_failure`.
  - `cli.rs`: `sync_prints_unshown_refusals_once` — a refusal file present; `tdo sync` stderr contains `Not saved: "<label>"`; second `tdo sync` does not.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_run && cargo test -p tdo --test cli` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS.
- [ ] **Step 5: Commit** `feat(sync): the runner drains the write queue`

### Task 7: TUI — queued saves, no stop prompt, overlay, drain on "sync ended"

**Files:**
- Modify: `crates/tdo/src/write.rs` (`Writer::run` uses `submit`/`undo_last`/`redo_last`; remove `Refused::Busy`, `stop_and_run`, `STOP_WAIT`, `running_for`, `set_waits`' stop part; `pub fn drain(&mut self, now) -> Vec<String>` returning refusal messages and calling `sync_after` when anything applied; `pub fn unshown(&mut self) -> Vec<String>`), `crates/tdo/src/term.rs` (remove the Busy/stop branch; after a queued result `reload`; `reload` builds with `queue::view`; on startup and in `poll_ended`: `writer.drain` then `reload`, show messages), `crates/tdo/src/app.rs` (remove the stop prompt state and text at ~1116; `:recover` Busy message), `crates/tdo/src/ui/tasks.rs` (` (queued)` suffix, muted style, when `is_pending`), `crates/tdo/src/ui/status.rs` (`pending + queued`)
- Modify docs: `docs/tui.md` (status line count includes queued edits, the ` (queued)` marker, the removed stop prompt, `:recover` while syncing)
- Test: `crates/tdo/tests/write.rs`, `crates/tdo/tests/term.rs`, `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `Store::{submit, drain, undo_last, redo_last}`, `Submitted`, `queue::view`, `Queue::{unshown, mark_shown}`.
- Produces: `Writer::run` → `Ok(message)`: applied: today's success text; queued: `<success text> · queued (a sync is running)`; cancelled: `undone: <title>`. Refusal text for a drained item (Decisions above). The undo-title stacks stay in step with the store's `Step`s.

- [ ] **Step 1: Write the failing tests:**
  - `write.rs`: `save_during_a_sync_is_queued_at_once` (lock held; `run` returns within 1 s with `queued` in the message; no trigger call); `drain_reports_refusals_with_the_title` (`Not saved: "<title>" changed on another device`); `drain_after_applied_items_triggers_batching`; `undo_of_a_queued_save_cancels_it`; delete the `stop_and_run` / `Refused::Busy` tests.
  - `term.rs`: `queued_edit_shows_after_save` (row shows the new title and ` (queued)` while the lock is held); `sync_ended_drains_and_reloads` (queue, release, write `ended`, `poll_ended` → row without marker, file edited); `startup_shows_unshown_refusals`.
  - `app_keys.rs`: no `stop it and save` prompt exists (remove its tests); `recover_while_syncing_says_try_again`.
  - `ui_render.rs`: status line `3 pending` with 1 unsynced + 2 queued; marker rendered.
- [ ] **Step 2: Run** `cargo test -p tdo` — expect FAIL.
- [ ] **Step 3: Implement**; update `docs/tui.md`; `python3 tools/check_links.py` exits 0.
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check` — expect PASS.
- [ ] **Step 5: Commit** `feat(tui): queue saves during a sync instead of asking to stop it`

### Task 8: `tdo op` — queued result, `--wait`, refusals

**Files:**
- Modify: `crates/tdo/src/cli/op.rs` (`submit` with tool `"op"`; build its snapshot with `queue::view`; `--wait` flag on write subcommands; print unshown refusals on stderr first), `crates/tdo/src/main.rs` (flag)
- Modify docs: `docs/cli.md` if it exists, else the `tdo op` section in `docs/sync.md` (queued output, `--wait`, exit codes)
- Test: `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Produces: queued → stdout `queued (a sync is running): <queue id>`, exit 0. `--wait` → poll every 100 ms until the item is neither `.json` nor `.claim`: journal entry by `from_queue` → print its id, exit 0; refusal file → print the refusal on stderr, mark shown, exit with the existing blocked/stale code (`op.rs:42`). Undo by id (`tdo op undo <id>`) through `Store::undo` (Task 5).

- [ ] **Step 1: Write the failing tests:** `op_during_a_sync_is_queued` (hold the vdir lock from the test process; `tdo op complete …` exits 0 within 1 s, stdout starts with `queued (a sync is running): `); `op_wait_reports_the_outcome` (spawn with `--wait`, release the lock after 300 ms → exit 0, stdout is a journal id); `op_wait_reports_a_refusal` (change the file before releasing → blocked/stale exit code, stderr `Not saved`); `op_prints_unshown_refusals_once`.
- [ ] **Step 2: Run** `cargo test -p tdo --test cli_op` — expect FAIL.
- [ ] **Step 3: Implement**; update the docs page.
- [ ] **Step 4: Run** `cargo test --workspace` — expect PASS.
- [ ] **Step 5: Commit** `feat(op): queue writes during a sync; --wait for the outcome`

### Task 9: Docs and close-out

**Files:**
- Modify: `docs/sync.md` (queue: when an edit is queued, drain points, refusals, interaction with batching), `docs/architecture.md` (`queue/` layout, claim/cancel, `from_queue`, overlay, prepare/commit), `docs/superpowers/specs/2026-11-06-write-queue-design.md` (one line under §6.2: the item stores the prepared entry, see this plan), `AGENTS.md` (plan 2 done; next plan 3: rebasing behind `rebase-merge`)

- [ ] **Step 1:** Update the docs; `python3 tools/check_links.py` exits 0.
- [ ] **Step 2: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check` — expect PASS.
- [ ] **Step 3: Manual check, synthetic test setup only, after asking the user** (`XDG_CONFIG_HOME=~/twydo-phase0/xdg`, list "Twydo test"; never production): start `tdo sync` with a slow vdirsyncer (or hold the lock with `flock` on `<vdir-state>/lock`), edit two tasks in the TUI → both show ` (queued)` at once, status `2 pending`; release → markers clear, files edited, one later sync sends them. Record in the PR description.
- [ ] **Step 4: Commit** `docs: describe the write queue` and open the PR (references #38, plan 2 of 3).
