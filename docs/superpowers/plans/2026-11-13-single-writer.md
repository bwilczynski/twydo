# Single Writer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Replace the per-session, flag-driven `twydo_core::store::Store` with a pure planner, one daemon-owned `Writer` and a per-session `History`.

**Architecture:** `store::plan::plan` gates and plans an operation with no I/O. `store::writer::Writer` (one per daemon, `&self`, no per-call state) owns the journal and queue; everything that touches the vdir is a method of `Locked`, obtained from the writer with the vdir lock. `store::history::History` holds one undo/redo list per session. The daemon decides commit-or-queue from the lane.

**Tech Stack:** Existing Rust workspace (edition 2024). No new dependencies.

**Spec:** [One writer for the vdir](../specs/2026-11-13-single-writer-design.md). Read it before any task.

## Global Constraints

- Work in the worktree `~/.herdr/worktrees/twydo/feat-architecture` (branch `feat/architecture`), addressed by absolute path. No push, no PR, no release.
- A refactor: no gate decision, no on-disk format and no RPC shape changes. The only behavior change is Task 1's fix.
- Durable formats are frozen: journal entry format 1, queue `Item`/`Prepared`/`Kind` serde forms, `<id>.claim`, `cancelled/`, `refused/`, the collection-operation fence.
- Durable step order is a requirement (spec §4.2): commit = entry `prepared` → file writes → stage `applied`; drain of one item = rename to `.claim` → commit naming `from_queue` → claim removed; cancel = `cancelled/<id>.json` written → `<id>.json` removed; refusal = `refused/<id>.json` written → claim removed.
- The wait for an outside holder of the vdir lock is 250 ms (`TRY_LOCK`).
- Existing tests in `crates/twydo-daemon/tests`, `crates/tdo/tests` and the process-kill tests are the contract: do not edit their assertions. A test may change only where it constructs or calls `Store` directly, or where a task below names it.
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; never touch a production daemon, config, phone or server. No personal data in the repository.
- Cargo target dir must not be under `/tmp` (use the default `target/` in the worktree).
- Each task ends green: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`.
- Commits pass commitlint (`type(scope): subject`, lowercase subject start, no trailing full stop, header ≤ 100, body lines ≤ 100). No agent attribution, no `Co-Authored-By`, no session links.
- Do not `git add -A`; add explicit paths.

## Review Focus

1. A queued item or journal entry written by 0.3.1 no longer decodes → Task 3 adds a decode test over literal 0.3.1 JSON.
2. A crash window moves because a durable step was reordered → Task 3 keeps the order table; the existing recovery and process-kill tests must pass unedited.
3. An edit saved while an outside process holds the vdir lock is stranded in the queue → Task 6 adds a test for it.
4. An undo after a drain refused a queued step names the wrong step or errors → Task 6 keeps `an_undo_names_the_step_it_reverted_when_a_drain_refuses_one_meanwhile` and `undo_skips_a_refused_queued_edit` green before deleting the pruning.
5. A live time-zone change is half-applied at a drain (planned "today" vs drained "today") → Task 6 adds a test.

---

## File map

- `crates/twydo-core/src/store/plan.rs` (new): `Policy`, `plan`, the internal `Planner`, `Plan`, `Target`.
- `crates/twydo-core/src/store/{family,order,reparent}.rs`: planners become `impl Planner<'_>`; commit-time checks move to the writer.
- `crates/twydo-core/src/store/writer.rs` (new): `Writer`, `Locked`, `Ctx`, `Fence`.
- `crates/twydo-core/src/store/history.rs` (new): `History`, `Step`, `StepState`, `Access`.
- `crates/twydo-core/src/store/mod.rs`: shrinks to module wiring, `WriteError`, state-dir helpers, `Submitted`, `Drained`, `DrainOutcome`.
- `crates/twydo-core/src/store/retired.rs`, `sync/run.rs`, `sync/resolve.rs`, `store/queue.rs`: take the writer.
- `crates/twydo-daemon/src/server/{mod,writes,sessions,admin,lists,syncjob,jobs}.rs`: one `Shared.writer`, `with_vdir`, sessions hold a `History`.
- `crates/twydo-core/tests/common/store.rs` (new): the test wrapper that keeps `apply`/`submit`/`undo_last`/`redo_last`/`drain` call shapes.

Names used across tasks (exact):

```rust
// store/plan.rs
#[derive(Clone, Copy)]
pub struct Policy<'a> { pub matrix: &'a Matrix, pub mode: WriteMode }
pub fn plan(snap: &Snapshot, op: &Op, policy: Policy<'_>, now: DateTime<Utc>) -> Result<Prepared, WriteError>;

// store/writer.rs
pub struct Ctx<'a> { pub now: DateTime<Utc>, pub request: Option<RequestRef>, pub config: &'a Config }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fence { Now, AtDrain }
impl Writer {
    pub fn open(cfg: &Config, matrix: Matrix) -> Result<Writer, WriteError>;   // no recovery
    pub fn policy(&self) -> Policy<'_>;
    pub fn state_dir(&self) -> &Path;
    pub fn lock(&self, wait: Duration) -> Result<Option<Locked<'_>>, WriteError>;
    pub fn lock_waiting(&self) -> Result<Locked<'_>, WriteError>;              // blocks
    pub fn with_lock<'a>(&'a self, held: &'a VdirLock) -> Locked<'a>;
    pub fn enqueue(&self, kind: Kind, label: &str, origin: &str, fence: Fence, ctx: &Ctx<'_>) -> Result<Item, WriteError>;
    pub fn revert_item(&self, entry: &str) -> Result<Kind, WriteError>;        // Kind::Revert for `entry`
    pub fn cancel(&self, item: &Item, ctx: &Ctx<'_>) -> Result<bool, WriteError>;
}
impl Locked<'_> {
    pub fn writer(&self) -> &Writer;
    pub fn recover(&self) -> Result<RecoveryReport, WriteError>;
    pub fn commit(&self, p: Prepared, ctx: &Ctx<'_>) -> Result<String, WriteError>;
    pub fn revert(&self, entry: &str, ctx: &Ctx<'_>) -> Result<String, WriteError>;
    pub fn restore(&self, path: &str, expect: Option<&str>, content: Option<&str>, uids: Vec<String>, ctx: &Ctx<'_>) -> Result<String, WriteError>;
    pub fn abandon(&self, entry: &str) -> Result<bool, WriteError>;
    pub fn drain(&self, ctx: &Ctx<'_>) -> Result<Vec<Drained>, WriteError>;
}

// store/history.rs
pub struct Step { pub title: String, pub state: StepState }
pub enum StepState { Applied(String), Queued(Item), Cancelled(Item) }
pub enum Access<'a> {
    /// The vdir may be free: lock (waiting at most `wait`) only when a step needs it;
    /// queue with `fence` when someone outside holds it.
    Try { writer: &'a Writer, wait: Duration, fence: Fence },
    /// A long job holds the vdir: never lock, always queue.
    Queue(&'a Writer, Fence),
}
impl History {
    pub fn new() -> History;
    pub fn record(&mut self, title: String, s: &Submitted);
    pub fn forget_applied(&mut self, entry: &str);   // an entry reverted by id, outside the history
    pub fn undo(&mut self, access: Access<'_>, origin: &str, ctx: &Ctx<'_>, drained: &mut Vec<Drained>) -> Result<(Submitted, Step), WriteError>;
    pub fn redo(&mut self, access: Access<'_>, origin: &str, ctx: &Ctx<'_>, drained: &mut Vec<Drained>) -> Result<(Submitted, Step), WriteError>;
}
```

`History` takes the vdir lock itself, lazily (amended after Task 4's review): a queued step is
cancelled with no lock, an applied step waits `wait` for it, a cancelled step's redo probes
once without waiting. Every drain it runs is appended to `drained`, also on `Err`.
`Writer::push(&Item, Fence)` (Task 3) queues an existing item unchanged.

`History::undo`/`redo` return the step that moved (its `title`, and the id it named before the
move, are what the daemon puts in `undid`). An empty list is `WriteError::NothingToUndo` /
`NothingToRedo`, as today.

---

### Task 1: The sync runner's drain uses the daemon's matrix

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs` (`run`, `drain_into` and their callers inside the file), `crates/twydo-core/src/store/queue.rs` (`drain_with_lock`), `crates/twydo-core/src/store/mod.rs` (`for_drain`)
- Modify: `crates/twydo-daemon/src/server/syncjob.rs`, `crates/twydo-daemon/src/server/lists.rs:326`
- Test: `crates/twydo-daemon/tests/queue.rs`

**Interfaces:**
- Produces: `pub fn run_with(cfg: &Config, matrix: &Matrix, opts: &RunOptions, now: DateTime<Utc>) -> Result<RunReport, SyncError>` in `sync::run`; `run(cfg, opts, now)` stays and calls it with `Matrix::embedded()`. `queue::drain_with_lock(cfg, matrix: &Matrix, held, now)`.

- [x] **Step 1: Write the failing test** `a_drain_by_the_sync_runner_uses_the_daemons_matrix` in `crates/twydo-daemon/tests/queue.rs`. Start the daemon with `[sync]` configured (`Fixture::with_sync`, `fake_vdirsyncer`) and, through `TestDaemon::start_cfg`'s options closure, `opts.matrix` set to a custom matrix that refuses a title edit which the embedded matrix admits in the fixture's write mode (build it from the embedded matrix's TOML with the `edit-title.resource` contract's verdict changed to `fail`; follow how other daemon tests build a custom matrix, e.g. in `tests/reports.rs` or `tests/writes.rs`). Hold the lane long with `handle().hold_long(LongKind::Sync)`, submit `edit_title` (it is queued: assert `WriteOutcome::Queued`), release the guard, then run a sync (`sync.run`) while the item is still queued. If the release drain takes the item first, instead make the fake vdirsyncer block on a file so the edit is submitted during the real sync job; model that on `an_edit_queued_while_the_drain_runs_is_drained_after_the_job`. Assert the item's `op.finished` state is refused and the file's `SUMMARY` is unchanged.
- [x] **Step 2: Run it** — `cargo test -p twydo-daemon --test queue a_drain_by_the_sync_runner_uses_the_daemons_matrix`. Expected: FAIL, the edit is applied. If it passes because a daemon-side drain (which already uses `shared.matrix`) always wins the race, move the test to `crates/twydo-core/tests/sync_run.rs` as `runner_drain_uses_the_given_matrix`, calling `run_with` with the refusing matrix over a queue item pushed as in `runner_drains_before_syncing`; expected failure then is "no function `run_with`".
- [x] **Step 3: Implement** `run_with` and thread `matrix` to `drain_into` → `drain_with_lock` → `Store::without_recovery(cfg, matrix.clone())`. Remove `Store::for_drain`. In the daemon pass `&shared.matrix` at both `run` call sites.
- [x] **Step 4: Run** the test (PASS), then `cargo test --workspace` and the lint command. Expected: all green.
- [x] **Step 5: Commit** `fix(daemon): gate edits drained during a sync with the daemon's matrix` (paths: the files above).

### Task 2: Extract the pure planner

**Files:**
- Create: `crates/twydo-core/src/store/plan.rs`
- Modify: `crates/twydo-core/src/store/{mod,family,order,reparent}.rs`
- Test: `crates/twydo-core/tests/plan.rs` (new)

**Interfaces:**
- Produces: `Policy`, `plan` (File map). Internal: `pub(super) struct Planner<'a> { pub vdir: &'a Path, pub policy: Policy<'a> }` holding today's `Store::plan`, `plan_save`, `target`, `rel`, `text_edit_head` and every `plan_*` method of `family.rs`, `order.rs`, `reparent.rs`. `Plan` and `Target` move with it.
- `Store::prepare` becomes: fence check (`ensure_idle` unless `fence_at_drain`), then `plan::plan(snap, op, Policy { matrix: &self.matrix, mode: self.mode }, now)`.

- [x] **Step 1: Write the failing tests** in `crates/twydo-core/tests/plan.rs`, each building a `Snapshot` from `common` helpers and calling `plan` with `Matrix::embedded()` in the helpers' usual write mode. No `Store`, and assert no state directory is created:
  - `plan_of_a_title_edit_writes_one_file_and_takes_no_state`: `Op::Edit` with `Change::Summary`; `prepared.entry.writes.len() == 1`, `before` equals the loaded bytes, `after` contains the new `SUMMARY`; the config's `state_dir()` does not exist afterwards.
  - `plan_of_a_family_completion_holds_every_member`: completing a checklist with two items; every member path is in `writes` or `holds`.
  - `plan_refuses_what_the_gate_refuses`: an op the gate blocks (reuse a refusal case from `tests/gate.rs`) returns `WriteError::Blocked` with the same reason string as `gate::check`.
  - `plan_matches_store_prepare`: for the title edit, `plan(..)` equals what `Store::apply` journals (compare `writes` of the journal entry).
- [x] **Step 2: Run** `cargo test -p twydo-core --test plan`. Expected: FAIL to compile, `plan` missing.
- [x] **Step 3: Implement.** Move, do not rewrite: the bodies keep their logic; `self.vdir`/`self.matrix`/`self.mode` become `self.vdir`/`self.policy.matrix`/`self.policy.mode` on `Planner`. `plan` builds the `Planner` with `&snap.config.vdir`. If any existing test fails because a snapshot's `config.vdir` differs from the store's, stop and report it (do not paper over it). Commit-time methods (`check_family`, `check_holds`, `run`, …) stay on `Store`.
- [x] **Step 4: Run** `cargo test --workspace` and lint. Expected: green; no existing test edited.
- [x] **Step 5: Commit** `refactor(core): extract the pure operation planner`.

### Task 3: The writer

**Files:**
- Create: `crates/twydo-core/src/store/writer.rs`
- Modify: `crates/twydo-core/src/store/{mod,family,retired,queue}.rs`
- Test: `crates/twydo-core/tests/writer.rs` (new)

**Interfaces:**
- Produces: `Writer`, `Locked`, `Ctx`, `Fence` (File map).
- `Store` becomes a shell: it holds a `Writer` plus its stacks and flags, and every method delegates. Its public API and behavior do not change in this task (all existing tests pass unedited). Keep the internal drain-steps-with-errors function `pub(crate)` for the shell's `settle_queued`.

Behavior to carry over exactly, from `store/mod.rs` at the task's base:
- `Locked::commit` = `commit_locked` with `from_queue: None`, plus the collection-fence check (`ensure_idle`) first; the entry's `request` is `ctx.request`.
- `Locked::revert` = `revert_locked`; `restore`, `abandon` = today's bodies without taking the lock; `recover` = `journal::recover_locked`.
- `Locked::drain` = `drain_locked` (`drain_steps` + `refuse_fenced`), with `ctx.config` in place of `self.cfg` for the snapshot it gates on and `ctx.now` as `now`.
- `Writer::enqueue` builds the `Item` (`queue::new_id(ctx.now)`, `created: ctx.now`, `origin: queue::origin(origin)`, `request: ctx.request`), checks the fence only for `Fence::Now`, pushes. `Writer::revert_item` builds the `Kind::Revert` of `revert_or_queue`. `Writer::cancel` = `Queue::cancel_by(item, ctx.request.clone(), ctx.now)`.
- `Writer::lock(wait)` = `try_lock_for(wait)`; `lock_waiting` = `VdirLock::exclusive`.

- [x] **Step 1: Write the failing tests** in `crates/twydo-core/tests/writer.rs`:
  - `commit_under_the_lock_journals_and_applies`: `plan` a title edit, `writer.lock(Duration::ZERO)` is `Some`, `commit` returns an id whose entry is `Stage::Applied` and carries `ctx.request`.
  - `lock_is_none_while_an_outside_holder_has_it`: hold `VdirLock::exclusive` on the state dir; `writer.lock(Duration::from_millis(50))` is `Ok(None)`.
  - `enqueue_then_drain_applies_the_item_once`: `enqueue(Kind::Op{..}, "t", "daemon", Fence::Now, &ctx)`, then `lock` + `drain` returns one `Drained` with `DrainOutcome::Applied`, the entry names `from_queue`, a second `drain` returns empty.
  - `drain_gates_today_in_the_live_time_zone`: an item whose gate result depends on "today" is not needed; assert instead that `drain` reads the zone from `ctx.config` by draining with a config whose `timezone` differs from the writer's opening config and checking the entry was gated without error (smoke) — the daemon-level test in Task 6 pins the behavior.
  - `a_0_3_1_queue_item_and_journal_entry_still_decode`: two string literals, a queue item (`kind: "op"`, with `prepared`) and a journal entry, produced by the code at this task's base (generate them once with a scratch test, paste as literals with synthetic content); `serde_json::from_str::<Item>` and `::<Entry>` succeed and re-serialize to equal JSON values.
- [x] **Step 2: Run** `cargo test -p twydo-core --test writer`. Expected: FAIL to compile.
- [x] **Step 3: Implement** `writer.rs` by moving the bodies; turn `Store` into the shell. `Locked` holds either an owned `VdirLock` or a borrowed one.
- [x] **Step 4: Run** `cargo test --workspace` and lint. Expected: green; no existing test edited.
- [x] **Step 5: Commit** `refactor(core): add the single vdir writer under the store`.

### Task 4: The history

**Files:**
- Create: `crates/twydo-core/src/store/history.rs`
- Modify: `crates/twydo-core/src/store/mod.rs`
- Test: `crates/twydo-core/tests/history.rs` (new)

**Interfaces:**
- Consumes: `Writer`, `Locked`, `Ctx`, `Fence` from Task 3.
- Produces: `History`, `Step`, `StepState`, `Access` (File map). The `Store` shell replaces its `done`/`undone` vectors with one `History` (titles empty) and implements `undo_last`/`redo_last` on it: try the lock for `TRY_LOCK` unless `vdir_busy`; with it, `drain` then `History::undo(Access::Locked(..))`; without it, `History::undo(Access::Queue(writer, fence))` followed by the shell's existing second probe.

Rules of `undo`/`redo` (today's `undo_last`, `redo_last`, `revert_step`):
- `Applied(id)`: `Access::Locked` → `locked.revert(id)`; `Access::Queue` → `enqueue(writer.revert_item(id)?, label "undo"/"redo", origin, fence)`.
- `Queued(item)`: `writer.cancel` true → `Submitted::Cancelled`, the step becomes `Cancelled(item)`. Else, if a journal entry names the item, revert that entry as for `Applied`. Else, if the item is refused, drop the step and continue with the next. Else `WriteError::Busy` (a drainer is mid-commit).
- `Cancelled(item)`: enqueue a clone with a new id and `ctx.request`; under `Access::Locked` drain afterwards and report `Applied` if the drain applied it.
- A new `record` clears the redo list.

- [x] **Step 1: Write the failing tests** in `crates/twydo-core/tests/history.rs`:
  - `undo_then_redo_of_an_applied_step_round_trips_the_file`
  - `undo_of_a_queued_step_cancels_it_and_redo_queues_it_again_under_a_new_id`
  - `a_refused_queued_step_is_skipped_and_the_step_before_it_is_undone` (returned `Step.title` is the earlier step's title)
  - `an_empty_history_says_nothing_to_undo` (`WriteError::NothingToUndo`, `NothingToRedo`)
  - `a_new_record_clears_the_redo_list`
- [x] **Step 2: Run** `cargo test -p twydo-core --test history`. Expected: FAIL to compile.
- [x] **Step 3: Implement** `history.rs`; switch the shell to it and delete `Step` and `revert_step` from `mod.rs`.
- [x] **Step 4: Run** `cargo test --workspace` and lint. Expected: green; no existing test edited.
- [x] **Step 5: Commit** `refactor(core): keep a session's undo history as one list`.

### Task 5: Core callers take the writer

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs`, `sync/resolve.rs`, `store/queue.rs`, `store/retired.rs`, `store/mod.rs`
- Modify: `crates/twydo-daemon/src/server/{mod,syncjob,lists,admin}.rs`
- Modify (call sites only): `crates/twydo-core/tests/{sync_run,resolve,retired,lists_recovery}.rs`

**Interfaces:**
- Produces:
  - `sync::run::run_with(cfg: &Config, writer: &Writer, opts: &RunOptions, now)` (replaces Task 1's matrix parameter); `run(cfg, opts, now)` stays as a convenience that opens a `Writer` with the embedded matrix. The runner's drains are `writer.with_lock(&held).drain(&Ctx { now, request: None, config: cfg })`. `queue::drain_with_lock` is removed.
  - `sync::resolve::resolve(.., writer: &Writer, ..)` in place of `Store::open(cfg)`: it calls `writer.lock_waiting()?.restore(..)` where it called `writer.restore`; recovery still runs under the lock inside `restore`.
  - `store::recover_list(locked: &Locked<'_>, active: &Snapshot, backup_id: &str, to: &str, ctx: &Ctx<'_>) -> Result<(String, bool), WriteError>` as a free function (today `Store::recover_list`, minus taking the lock).
  - `Shared.writer: Writer` in the daemon, opened in `Daemon::start` with `opts.matrix`; `Shared.matrix` stays for the gate reports.
- The daemon's sessions still hold `Store` shells after this task; only the three core entry points above and their daemon call sites change. `lists.recover` takes `shared.writer.lock_waiting()` and records the result on the session as it does today (`s.pushed`).

- [x] **Step 1: No new test.** (Amended: the planned `resolve_does_not_run_journal_recovery` was wrong; every write recovers under the lock and that must not change.) The existing resolve, sync-run and retired tests are the contract.
- [x] **Step 2: Change the signatures** and see the call sites fail to compile.
- [x] **Step 3: Implement** the four interface changes and update call sites. In core tests, change only the call expressions (e.g. `resolve(.., &writer, ..)`, `recover_list(&locked, ..)`), never assertions.
- [x] **Step 4: Run** `cargo test --workspace` and lint. Expected: green, including Task 1's test.
- [x] **Step 5: Commit** `refactor(core): hand the sync runner, conflict resolution and list recovery the writer`.

### Task 6: The daemon writes through one writer

**Files:**
- Modify: `crates/twydo-daemon/src/server/{writes,sessions,admin,lists,mod,lane}.rs`
- Test: `crates/twydo-daemon/tests/{queue,writes,sync}.rs`

**Interfaces:**
- Consumes: `plan`, `Writer`, `Locked`, `Ctx`, `Fence`, `History`, `Access`, `Shared.writer`.
- Produces in `writes.rs`:
  - `pub(super) trait LaneHeld {}` implemented in `lane.rs` for `WriteGuard<'_>` and `LongGuard<'_>`.
  - `pub(super) fn with_vdir<T>(shared: &Shared, _lane: &impl LaneHeld, f: impl FnOnce(&Writer) -> T) -> Result<T, RpcError>`: takes the commit gate's read side (closed → the existing `stopping()` error), runs `f` with `&shared.writer`, releases the gate. Every daemon call that can commit, queue or drain goes through it, so each needs a lane guard to compile. Inside `f` the vdir lock is taken with `writer.lock(TRY_LOCK)` for a task write or a drain, by `History` (via `Access::Try { wait: TRY_LOCK, .. }`) for an undo or redo, and with `writer.lock_waiting()` for the paths that cannot be queued.
  - `Session { pub history: History }`; `Titled`, `Session.store`, `Session::pushed`, `Session::prune_refused`, `sessions::step`, `writes::prune_refused_steps` are removed. `Sessions::new` no longer needs the config or matrix.
  - `Ctx.config` is the publisher's current configuration (the live one), `Ctx.request` the `RequestRef` of the RPC.

Task write flow (spec §4.4), for `op.submit`; `op.undo`/`op.redo`/`journal.revert` follow the same table with `History::undo`/`redo` (lane free: `Access::Try { wait: TRY_LOCK, fence: Fence::Now }`; `LongBusy`: `Access::Queue` with the row's fence) or `Locked::revert` in place of plan + commit:

| Lane | Vdir lock | Action |
|---|---|---|
| `Ok(write guard)` | `Some(locked)` within 250 ms | `locked.drain(ctx)`, then `plan(view, op, writer.policy(), now)` and `locked.commit` |
| `Ok(write guard)` | `None` | `plan`, `writer.enqueue(Kind::Op{..}, label, "daemon", Fence::Now, ctx)`, then `shared.request_drain()` |
| `Err(LongBusy(Sync))` | not tried | `plan`, `enqueue(.., Fence::Now, ..)` |
| `Err(LongBusy(List))` | not tried | `plan`, `enqueue(.., Fence::AtDrain, ..)` |

The plan happens before the lane is taken where it does today (the view comes from `planning_view`). `journal.revert` uses no session history. `drain_passes` uses `with_vdir` and `writer.lock(TRY_LOCK)` and keeps its `DRAIN_PASSES`/`DRAIN_BUSY_TRIES` loop, treating a `None` lock as today's `WriteError::Busy`. Startup recovery (`server/mod.rs:574`) becomes `shared.writer.lock(Duration::ZERO)` → `recover()`, keeping "a busy vdir defers recovery" as today. `journal.abandon` and `journal.recover` use `with_vdir` and `writer.lock_waiting()` under their waiting lane.

- [x] **Step 1: Write the failing tests:**
  - `crates/twydo-daemon/tests/queue.rs::an_edit_saved_while_an_outside_process_holds_the_vdir_lock_is_queued_then_applied`: hold `VdirLock::exclusive(state)` from the test, `submit(edit_title)` answers `WriteOutcome::Queued` within 2 s, release the lock, the item's `op.finished` reports applied without any further request and the file holds the new title.
  - `crates/twydo-daemon/tests/sync.rs::a_live_time_zone_change_applies_to_a_queued_edit_at_its_drain`: with a fake clock or a fixed `now` near midnight UTC, queue (behind `hold_long`) a due-date edit whose admission or resulting date depends on "today" in the configured zone, rewrite `config.toml` with another `timezone`, wait for the reload, release; assert the drain's outcome matches a fresh submit made under the new zone. If no operation's outcome depends on "today" in a way a test can observe, write the test as `a_drain_reads_the_live_configuration` against `Ctx.config` being the publisher's configuration (assert via a `DaemonOptions` test hook that already exists, or report NEEDS_CONTEXT rather than inventing a hook).
- [x] **Step 2: Run** them. Expected: the first passes or fails depending on today's second probe (record which); the second fails or needs the interface. Both must pass after Step 3.
- [x] **Step 3: Implement** in this order, running `cargo test -p twydo-daemon` after each: (a) `LaneHeld`, `with_vdir`; (b) `run_submit`; (c) sessions → `History`, `run_undo`, `lists.recover`'s record; (d) `run_revert`, `run_abandon`, journal recovery, `drain_passes`, startup; (e) delete `run_store_write`, the setters' call sites and the pruning. Before deleting `prune_refused_steps`, run `cargo test -p twydo-daemon --test queue undo_ an_undo_` and confirm `undo_skips_a_refused_queued_edit` and `an_undo_names_the_step_it_reverted_when_a_drain_refuses_one_meanwhile` pass without it; if either cannot pass with lazy skipping, keep an eager prune over `History` (add `History::drop_refused(&mut self, queue: &Queue)`) and say so in the report.
- [x] **Step 4: Rewrite the lock-order comment** at the top of `writes.rs` to match: a task write is "request log slot → session mutex → lane → `with_vdir` (commit gate, then vdir lock) → publisher"; drop the paragraphs that describe pruning under the drain hook if the pruning is gone. Keep every other section accurate.
- [x] **Step 5: Run** `cargo test --workspace` and lint. Expected: green; no existing daemon or `tdo` test assertion edited.
- [x] **Step 6: Commit** `refactor(daemon): write through one writer behind the lane`.

### Task 7: Delete `Store`

**Files:**
- Create: `crates/twydo-core/tests/common/store.rs`
- Modify: `crates/twydo-core/src/store/mod.rs`, `store/queue.rs`; `crates/twydo-core/tests/*.rs` and `crates/twydo-daemon/tests/{admin,debt,queue,read}.rs` (construction and calls only)
- Modify: `docs/testing.md`

**Interfaces:**
- Removes from `twydo_core::store`: `Store` and all its methods, `submit_with`, `settle_queued`, the setters, `take_drained`, `Store::lock`/`try_lock_for`. `WriteError::Busy` stays (a drainer mid-commit during an undo).
- Produces `tests/common/store.rs`: `pub struct Store { pub writer: Writer, history: History, cfg: Config, request: Option<RequestRef>, busy: bool }` with the methods the core tests call — `open`, `open_with` (run `recover` under a zero-wait lock, return `(Store, RecoveryReport)`), `without_recovery`, `apply`, `submit`, `undo`, `undo_last`, `redo_last`, `drain`, `restore`, `abandon`, `recover_list`, `set_request`, `set_vdir_busy`, `state_dir` — built on `plan`, `Writer`, `History`. `submit` = lock within 250 ms → `drain` + `commit`, else `enqueue`; no second probe.

- [x] **Step 1: Write the wrapper** and switch every core test's `use twydo_core::store::Store` to `common::store::Store`. Daemon tests that open a `Store` use `Writer` directly.
- [x] **Step 2: Delete `Store`** from the library. `cargo build --workspace --all-targets` lists what still references it.
- [x] **Step 3: Run** `cargo test --workspace`. For each failing test decide: a call-shape problem → fix the call; a test of behavior that only existed between processes (the second lock probe via `submit_with`, lock-wait blocking, `take_drained` collection) → delete it and add a row to the cleanup table in `docs/testing.md` naming the retained owner (for the lost-wakeup tests: Task 6's `an_edit_saved_while_an_outside_process_holds_the_vdir_lock_is_queued_then_applied` and `an_edit_queued_while_the_drain_runs_is_drained_after_the_job`). Never delete a test whose assertion is about journal contents, file bytes, gate refusals or recovery; report it instead.
- [x] **Step 4: Run** `cargo test --workspace`, lint, `python3 tools/check_links.py`. Expected: green. `rg -n '\bStore::(open|open_with|without_recovery|for_drain)\b|set_vdir_busy|set_fence_at_drain|set_collect_drained|set_lock_wait|take_drained|submit_with|settle_queued' crates/*/src` prints nothing.
- [x] **Step 5: Commit** `refactor(core): remove the per-session store`.

### Task 8: Documentation

**Files:**
- Modify: `docs/architecture.md` (the write path, the write queue, the operation journal's Protocol and Recovery, Locking), `docs/testing.md` (owner table: add `plan`, `writer`, `history`), `AGENTS.md` (current work and roadmap), `docs/superpowers/plans/2026-11-13-single-writer.md` (tick the boxes)

- [x] **Step 1: Update `docs/architecture.md`** so it describes what the code does now: `Store::submit` → the daemon plans (`store::plan`), then commits under the lane and the vdir lock or queues; one `Writer` per daemon; drains use the daemon's matrix and live configuration; recovery call sites as implemented. Remove statements that are no longer true (search for `Store::submit`, `250 ms`, "TUI", "Those locks survive"); keep the 250 ms wait where it still applies.
- [x] **Step 2: Update `AGENTS.md`:** the operation-evidence plan is complete; the single-writer refactor is done; next is write-queue plan 3, which re-plans with `store::plan`. Keep the device and data boundaries unchanged.
- [x] **Step 3: Run** `python3 tools/check_links.py` and `mise run test` (or its four commands). Expected: green.
- [x] **Step 4: Commit** `docs: describe the single vdir writer`.

## Execution notes

- Task 3 dropped the test `drain_gates_today_in_the_live_time_zone`: `timezone` only takes effect after a daemon restart.
- Task 4 was redesigned during execution: `Access::Try` takes the lock lazily, only when a step needs it, and drained items go to a sink.
- Task 5 dropped the test `resolve_does_not_run_journal_recovery`.
- Task 6 dropped the live-time-zone-at-a-drain daemon test, for the same reason as Task 3.
- The one observable value change: `undid.op_id` for a redo of a cancelled step is now the withdrawn item's id (it was none).
- The final review's fix restored the fence-before-gate order in `run_submit` (`Writer::check_fence`, before `plan`), as before the refactor; undo, redo and `journal.revert` keep their old order.
- A known flaky test, seen once before Task 4: `crates/twydo-daemon/tests/sync.rs::stop_request_is_withdrawn_after_the_stopped_sync`.
