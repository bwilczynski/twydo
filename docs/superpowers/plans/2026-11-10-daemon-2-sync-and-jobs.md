# Daemon plan 2: sync, jobs and the scheduler — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The hidden `tdo daemon` runs syncs and list jobs on its vdir lane, schedules the
startup, periodic and batched syncs itself, and serves every remaining mutation (conflicts,
journal, sync switch, lists) plus `input.parse` and a live `config.toml`, so plan 3 can switch the
clients over without adding daemon features.

**Architecture:** One **job runner** thread per daemon takes work in FIFO order (client jobs and
background syncs) and runs each under the lane's long token, so syncs, list jobs and writes stay
one at a time; writes during a job are queued as in plan 1. Client jobs have durable records
(`<state>/daemon/jobs/<id>.json`) for `job.get` and `request_id` replay across restarts. A
**scheduler** thread decides when a background sync is due (startup, every 60 s, the batching
rule, after a recorded conflict choice, switch turned on) from an injectable clock and posts it
to the runner. Short vdir writes that are not task ops (conflict resolve, journal recover and
abandon) take the lane as a write that waits behind a running job.

**Tech Stack:** Rust 2024, as plan 1 (`std::thread`, `mpsc`, serde, `schemars` 1.x, `notify` 8,
`libc`, `signal-hook` 0.3 in `tdo`), `tempfile` for tests. No new crates.

**Spec:** [`docs/superpowers/specs/2026-11-09-daemon-design.md`](../specs/2026-11-09-daemon-design.md)
(§5, §6.5–§6.8, §7, §8, §13), and plan 1
[`2026-11-09-daemon-1-core-and-writes.md`](2026-11-09-daemon-1-core-and-writes.md) for the
crate layout, the lane, sessions, replay and the lock order in `crates/twydo-daemon/src/server/writes.rs`.

## Where this plan sits

| Plan | Delivers | Clients |
|---|---|---|
| 1 (merged, PR #78) | crate, protocol, schema, lifecycle, snapshot/status/events, watcher, task writes, queue | unchanged |
| **2 (this)** | plan-1 review debt; jobs and `job.get`; `sync.run`/`stop`/`set_auto`; scheduler; orphaned vdirsyncer at startup; shutdown during a sync; `conflicts.*`; `journal.*`; `lists.*`; `input.parse`; `config.toml` reload | unchanged |
| 3 | client interface, on-demand start, TUI and CLI over the socket, plan-1 deferred client items | switched |

**Clients do not change in this plan.** The TUI, `tdo sync`, the batch waiter and the sync timer
keep running their own syncs. The daemon must coexist with them: they meet at the runner's
`run.lock` (a background run that finds it held returns `Skip::Busy`, which the daemon reports as
skipped, never as an error) and at the vdir lock (`WriteError::Busy` → `unavailable`).

## Decisions this plan makes (builder's calls in the spec)

- **Every job takes a `request_id`**, `sync.run` included (the spec lists it only for `lists.*`):
  a job is a change, and `job.get {request_id}` needs it. A job method answers at once with
  `JobStarted { job_id }`; the outcome comes as `job.finished` and from `job.get`.
- **Job records** live in `<vdir-state>/daemon/jobs/<job_id>.json`, written when the job is
  accepted and rewritten when it starts and ends. A record still `queued` or `running` when a
  daemon starts is rewritten as `interrupted`. Records older than `retention_days` (the `[sync]`
  value, else 30) are pruned at startup. Job ids are `job-<UTC yyyymmddThhmmss>-<8 hex>`.
- **One job runner thread, FIFO.** Background syncs are not jobs (no record, no `job_id`); a
  background sync is not added while another background sync is queued or a manual one is queued
  (that one covers it). A manual `sync.run` during a sync waits its turn (spec §8).
- **Background syncs that would be skipped emit nothing.** Before running, the runner checks the
  switch and backoff (`switch::is_off`, `SyncStatus::next_attempt > now`); a skip found there sends
  no events. `sync.run` (manual) always runs and always reports.
- **Sync events:** `sync.started { job_id?, reason }` and `sync.finished { job_id?, reason,
  result, status_rev }`, published after the post-sync reload. `reason` is an open enum:
  `manual`, `startup`, `periodic`, `batch`, `resolve`, `switch_on`, `list`.
- **No `job.progress`** in this plan: the list adapter reports none. The event name stays free.
- **Sync wording moves to core.** `problems`, `describe`, `list_names` and the warning lines of
  `tdo/src/cli/sync.rs` move to `twydo_core::sync::report`, so `sync.finished` carries the same
  lines `tdo sync` prints. `tdo sync` calls the moved functions; its output is unchanged.
- **Periodic interval:** 60 s, `DaemonOptions::periodic`; no config key (YAGNI; `docs/config.md`
  changes in plan 3 only if a key is added then).
- **Scheduler clock:** `trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }` in
  `server/schedule.rs`, `SystemClock` by default, `DaemonOptions::clock`. The loop never sleeps
  longer than 5 s at a time and re-reads the clock, so a laptop waking from sleep syncs within 5 s.
- **Overlay without reading the vdir during a long job.** While the lane is held long, the
  queue overlay is published from the publisher's last loaded files (the job's own reload replaces
  it when the job ends), so a queued row shows pending at once instead of waiting on the vdir
  lock held by the sync (plan-1 deferred item).
- **`Lane::write_waiting`** (new): a write token that waits through a long job. Used by
  `conflicts.resolve`, `journal.recover` and `journal.abandon`; never by `op.*`, which queue.
- **Conflict resolutions record their request.** A conflict gains an optional
  `resolved_by: { request, resolution, op_id }`; `ConflictStore::by_request` answers replays after
  a restart. `Resolution` gains serde derives (`snake_case`).
- **`journal.abandon` is idempotent** (an entry already `Abandoned` answers `{abandoned: false}`),
  so its in-memory replay is enough. `journal.recover` takes no `request_id` (it changes nothing a
  second run would repeat). `journal.revert` is a task write: it queues during a job and replays
  from the journal and queue like `op.submit`.
- **All list actions are jobs** (`LongKind::List`), including `lists.preview_delete` (it writes a
  backup) and `lists.export`. Reads (`lists.backups`, `lists.backup_preview`,
  `lists.preview_forget`) are plain methods. `lists.delete` keeps today's refusal (the core's
  `delete_list` refuses); the job fails with that message.
- **The collection fence moves to the drain, for edits saved during a list job only.** Every
  other write is unaffected: the gate still runs when the edit is saved (plan 1). `Queue::push`
  no longer calls `lists::operation::ensure_idle`, so an edit saved while a list job runs is
  queued instead of refused at once; `Store::apply` still checks the fence, so if the job leaves
  its operation unfinished, the edit is refused at drain with today's reason (spec §7.4). List
  jobs are rare, so the late refusal is accepted.
- **Config reload** watches the config file's directory (editors replace files by rename). The
  binding fields `vdir`, `state_dir`, `writes`, `sync` never change at run time; a change to them
  is reported in `status.config.restart_needed` and not applied. Every other field applies and is
  published with cause `config`. An unreadable or invalid file keeps the last good configuration
  and sets `status.config.error`.
- **Shutdown during a job:** writes and new jobs answer `unavailable`; queued jobs become
  `interrupted`; a running sync gets `DaemonOptions::shutdown_grace` (default 10 s) and is then
  stopped through `running::request_stop`; a running list job is waited for. `daemon.shutdown
  {when_idle: true}` refuses new jobs, lets queued ones finish, then stops.
- **Second `SIGINT`/`SIGTERM` exits at once** with code 130 (`signal_hook::flag::register_conditional_shutdown`);
  a vdirsyncer left behind is reaped by the next start (Task 6).

## Global Constraints

- Contracts in the main spec §11 bind everything; "nothing is ever planned against a newer
  snapshot than the client saw" (daemon spec §6.6).
- On-disk data unchanged except **optional** fields (`serde(default, skip_serializing_if =
  "Option::is_none")`): `Conflict.resolved_by`; new files only under `<vdir-state>/daemon/`.
- Pure modules (`ical`, `model`, `views`, `dateinput`, `repeatinput`, `edit`, `gate`) stay free of
  file, process and clock access.
- Only the daemon reads `config.toml`; nothing on `twydo-daemon`'s client side may call
  `Config::load` or `resolve_config`.
- Every test that starts a daemon sets its own socket; **no test runs the real `vdirsyncer`**:
  any config with `[sync]` uses a fake script (`tests/common::fake_vdirsyncer`), and
  `TestDaemon` turns the scheduler off unless the test turns it on.
- Mutating methods require a non-empty `request_id` and replay before any session, epoch or
  `based_on` check (plan 1's `replayed` order).
- Enums that clients receive are open (`#[schemars(transform = super::open_enum)]`, an
  `#[serde(other)] #[schemars(skip)] Unknown` variant): every new one in this plan.
- Error kinds stay plan 1's set; a vdir held by another process maps to `unavailable` with
  "the vdir is busy; try again".
- Message wording follows today's (`tdo sync`, `tdo conflicts resolve`, `tdo op`, the TUI).
- No personal data in the repository: fixtures are synthetic.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass at every commit; `schema/tdo.openrpc.json` is regenerated
  (`TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`) in every task that adds a method or
  type, and the breaking-change test against `schema/released.openrpc.json` passes. Conventional
  commits, no agent attribution lines.

## Review Focus

1. **A failing sync must back off, not loop.** With edits due and vdirsyncer failing, the
   scheduler runs one sync, then waits for `next_attempt`. Test:
   `failing_batch_sync_backs_off_instead_of_looping` (Task 5).
2. **Waking from sleep runs one catch-up sync, not one per missed minute.** Test:
   `clock_jump_runs_one_periodic_sync` (Task 5).
3. **`sync.stop` with nothing running must not hold off the next sync** (the stop file is
   fresh for 30 s). Test: `stop_with_nothing_running_does_not_block_the_next_sync` (Task 4).
4. **A daemon killed during a job** must not leave `job.get` saying `running` forever, and a
   resent request must not start the job twice. Test:
   `job_cut_short_by_kill_is_interrupted` (Task 6).
5. **A `config.toml` saved half-edited** keeps the last good configuration, says so in the
   status, and recovers when fixed. Test: `broken_config_keeps_the_last_good_one` (Task 9).

---

## File structure

```
crates/twydo-daemon/src/
  protocol/methods.rs      + ShutdownParams unchanged; StatusView gains sync.job_id, config, queue_unreadable
  protocol/jobs.rs         NEW JobStarted, JobGetParams, JobView, JobState, JobKind, JobResult
  protocol/sync.rs         NEW SyncRunParams, SyncSetAutoParams, SyncStopResult, SyncReason, SyncResult, SyncOutcome
  protocol/admin.rs        NEW conflicts.*, journal.*, input.parse params/results
  protocol/lists.rs        NEW WireListState, list params, backup views
  protocol/events.rs       + SyncStarted, SyncFinished, JobFinished
  server/jobs.rs           NEW JobStore (records), JobRunner (thread, FIFO), Work
  server/syncjob.rs        NEW run_sync: lane, events, runner report, drained items
  server/schedule.rs       NEW Clock, SystemClock, SchedState, next(), Scheduler thread
  server/admin.rs          NEW conflicts.*, journal.*, input.parse handlers
  server/lists.rs          NEW lists.* handlers and job bodies
  server/config.rs         NEW config watch and reload
  server/lane.rs           + write_waiting
  server/publish.rs        + live config, overlay-only publish, sync/config status
  server/mod.rs            + runner, scheduler, shutdown grace, accept backoff, orphan step
  server/dispatch.rs       + new method arms
  schema.rs                + METHODS rows
  tests/common/mod.rs      + fake_vdirsyncer, Fixture::with_sync, wait_for_event, scheduler off
  tests/jobs.rs tests/sync.rs tests/schedule.rs tests/admin.rs tests/lists.rs tests/config.rs
  tests/debt.rs            plan-1 deferred tests
crates/twydo-core/src/
  sync/report.rs           NEW moved wording from tdo/src/cli/sync.rs
  sync/running.rs          + reap_orphan
  sync/resolve.rs          + resolve_with (request), Resolution serde
  conflicts.rs             + ResolvedBy, Conflict.resolved_by, ConflictStore::by_request
  store/mod.rs             abandon idempotent
  store/queue.rs           push without fence; state() errors on an unreadable item; unreadable()
crates/tdo/src/cli/sync.rs      uses sync::report
crates/tdo/src/cli/daemon.rs    second signal exits
```

---

### Task 1: Plan-1 review debt

Fixes and tests the plan-1 reviews deferred here, except the ones that need the scheduler (the
Busy-drain retry, Task 5) or the overlay change (Task 2).

**Files:**
- Modify: `crates/twydo-daemon/src/server/mod.rs` (`accept_loop`), `crates/tdo/src/cli/daemon.rs`,
  `crates/twydo-core/src/store/queue.rs`, `crates/twydo-daemon/src/server/publish.rs`,
  `crates/twydo-daemon/src/protocol/methods.rs`
- Test: `crates/twydo-daemon/tests/debt.rs`, `crates/twydo-core/tests/queue.rs`,
  `crates/tdo/tests/daemon_process.rs`

**Interfaces:**
- Produces: `Queue::unreadable(&self) -> io::Result<Vec<PathBuf>>` (item files that do not
  parse); `Queue::state(id)` returns `Err(InvalidData, "unreadable queue item <path>: <e>")` when
  `id`'s own file does not parse; `StatusView.queue_unreadable: Vec<String>` (paths,
  `serde(default, skip_serializing_if = "Vec::is_empty")`).

- [ ] **Step 1: Write the failing tests**

In `tests/debt.rs` (use plan 1's `TestDaemon`, `submit`, `hold_long` helpers):
- `drain_error_is_reported_and_cleared`: hold the lane long, queue an edit, leave a `Prepared`
  journal entry for an unrelated uid (write the JSON as `tests/queue.rs` does), drop the guard →
  `status.drain_error` is `Some` containing `"unfinished"`; abandon the entry with
  `Store::abandon`, hold and drop the lane again → `drain_error` is `None` and the edit applied.
- `refusal_at_drain_with_the_session_gone_lands_in_status`: queue a stale edit from client A,
  close A and expire its session (`session_ttl = 50ms`), drop the guard → `status.refusals` has
  one entry with A's `request_id`.
- `already_applied_item_drains_as_satisfied`: queue `edit_title(uid, "X")`, write a file whose
  title is already `"X"` from outside, drop the guard → `op.finished` state `satisfied`.
- `queued_reply_without_a_job_triggers_a_drain`: covers the branch in `writes::queued` that
  calls `request_drain` when no long job holds the lane at reply time. Order it with a new
  `#[doc(hidden)] DaemonOptions::before_queued_reply: Option<TestHook>` (runs after the item is
  pushed, before the reply is built): the hook drops the test's long guard and waits until the
  drain hook has run → the item is applied within 2 s with no other trigger.
- `startup_drain_skips_a_held_lane`: start unready with `before_recovery` taking
  `lane_for_test().long(Test)` on another thread for 300 ms; leave a queued item on disk → the
  daemon becomes ready, and the item is applied once the guard drops.
- `refresher_panic_does_not_stop_later_refreshes`: a `#[doc(hidden)]
  DaemonOptions::status_panic_once: bool` makes the first `refresh_status` panic → a later
  `refusals.ack` still bumps `status_rev`.

In `twydo-core/tests/queue.rs`:
- `unreadable_item_is_reported_not_skipped`: write `queue/<id>.json` with `{` → `state(id)` is
  `Err` with kind `InvalidData` and the path in the message; `unreadable()` lists it;
  `by_request("x")` is `Ok(None)` (the other items still answer).

In `crates/tdo/tests/daemon_process.rs`:
- `second_sigint_exits_at_once`: start `tdo daemon`, wait for its socket, send `SIGINT` twice
  10 ms apart → the process has exited within 2 s, with status 130 or 0 (0 when teardown
  finished before the second signal).

- [ ] **Step 2: Run them; the queue, refresher and SIGINT tests fail**

Run: `cargo test -p twydo-daemon --test debt && cargo test -p twydo-core --test queue unreadable && cargo test -p tdo --test daemon_process second_sigint`
Expected: FAIL in `unreadable_item_is_reported_not_skipped`,
`refresher_panic_does_not_stop_later_refreshes` (no option), `second_sigint_exits_at_once`.
The other `debt.rs` tests may pass already: they pin plan-1 behaviour that had no test.

- [ ] **Step 3: Implement**

- `accept_loop`: on an `accept` error other than `WouldBlock`/`Interrupted`, log once per
  distinct error kind and sleep 100 ms before polling again (EMFILE no longer spins).
- `cli/daemon.rs`: register each signal with `flag::register_conditional_shutdown(sig, 130,
  Arc::clone(&stop))` before `flag::register(sig, Arc::clone(&stop))`.
- `Queue::state`, `Queue::unreadable` as in Interfaces; `build_status` fills `queue_unreadable`
  and logs each path once.
- `status_panic_once` on `DaemonOptions` and `Shared`, checked at the top of `run_refresher`'s loop.

- [ ] **Step 4: Run the tests and the workspace**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "fix(daemon): back off on accept errors, exit on a second signal, report unreadable queue items"
```

---

### Task 2: Publish the queue overlay without reading the vdir during a long job

**Files:**
- Modify: `crates/twydo-daemon/src/server/publish.rs`, `crates/twydo-daemon/src/server/mod.rs`
  (`run_refresher`)
- Test: `crates/twydo-daemon/tests/queue.rs`

**Interfaces:**
- Produces: `Publisher::overlay(&self, cause: Cause, uids: Vec<String>) -> io::Result<u64>`:
  rebuilds the snapshot from the last loaded `LoadReport` (kept by `reload`) and the current
  queue, without the vdir lock; publishes only if it differs.

- [ ] **Step 1: Write the failing test**

`queued_row_is_pending_while_the_vdir_lock_is_held`: hold the lane long and take
`VdirLock::exclusive(state)` on the test thread (as a sync's reconcile step does); submit an edit
→ answered `queued`, and within 1 s a `data.changed` arrives whose snapshot has the row
`pending: true`; release both.

- [ ] **Step 2: Run it**

Run: `cargo test -p twydo-daemon --test queue queued_row_is_pending_while`
Expected: FAIL (no `data.changed` within 1 s: the refresher waits for the vdir lock).

- [ ] **Step 3: Implement**

`reload` keeps the `LoadReport` it loaded (`State.last_report`). `overlay` takes `reload_lock`,
builds `queue::view(last_report.clone(), cfg)`, and publishes like `reload`. `run_refresher`
calls `overlay` instead of `reload(Cause::Op, …)` while `lane.current_long().is_some()`.

- [ ] **Step 4: Run the daemon tests**

Run: `cargo test -p twydo-daemon`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-daemon
git commit -m "fix(daemon): show queued edits as pending while a sync holds the vdir"
```

---

### Task 3: Job records, the job runner, `sync.run` and `job.get`

**Files:**
- Create: `crates/twydo-daemon/src/protocol/jobs.rs`, `crates/twydo-daemon/src/protocol/sync.rs`,
  `crates/twydo-daemon/src/server/jobs.rs`, `crates/twydo-daemon/src/server/syncjob.rs`,
  `crates/twydo-core/src/sync/report.rs`, `crates/twydo-daemon/tests/jobs.rs`,
  `crates/twydo-daemon/tests/sync.rs`
- Modify: `protocol/events.rs`, `protocol/mod.rs`, `server/mod.rs`, `server/dispatch.rs`,
  `schema.rs`, `crates/tdo/src/cli/sync.rs`, `tests/common/mod.rs`

**Interfaces:**
- Produces (protocol):
  - `JobStarted { job_id: String }`; `JobGetParams { job_id: Option<String>, request_id: Option<String> }` (exactly one).
  - `JobKind` (open, snake_case): `Sync, ListCreate, ListUpdate, ListArchive, ListUnarchive,
    ListPreviewDelete, ListDelete, ListReconcile, ListExport, ListForget, ListRecover`.
  - `JobState` (open, `tag = "state"`, snake_case): `Queued, Running, Finished { result: JobResult },
    Failed { kind: ErrorKind, message: String }, Interrupted`.
  - `JobView { job_id, request_id: Option<String>, kind: JobKind, state: JobState, created: DateTime<Utc>, ended: Option<DateTime<Utc>> }` — the result of `job.get`.
  - `JobResult` (open, `tag = "type"`, snake_case): `Sync(SyncResult)`; later tasks add their variants.
  - `SyncRunParams { request_id: String, force_delete: Vec<String> }` (list names, as `tdo sync --force-delete`).
  - `SyncReason` (open): `Manual, Startup, Periodic, Batch, Resolve, SwitchOn, List`.
  - `SyncOutcome` (open): `Ok, Failed, Stopped, Skipped`.
  - `SyncResult { outcome: SyncOutcome, problems: Vec<String>, warnings: Vec<String>, notes: Vec<String>, new_conflicts: usize, drained: usize }`.
  - `Event::SyncStarted { job_id: Option<String>, reason: SyncReason }`,
    `Event::SyncFinished { job_id: Option<String>, reason: SyncReason, result: SyncResult, status_rev: u64 }`,
    `Event::JobFinished { job_id: String, request_id: Option<String>, state: JobState }`.
- Produces (server):
  - `JobStore::open(state: &Path) -> io::Result<JobStore>`; `create(&self, kind, request: Option<RequestRef>) -> io::Result<JobView>`;
    `set_state(&self, id, JobState) -> io::Result<JobView>`; `get(id) -> io::Result<Option<JobView>>`;
    `by_request(request_id) -> io::Result<Option<JobView>>`; `interrupt_unfinished(&self) -> io::Result<usize>`;
    `prune(before: DateTime<Utc>) -> io::Result<usize>`.
  - `enum Work { Job { view: JobView, run: Box<dyn FnOnce(&Arc<Shared>) -> Result<JobResult, RpcError> + Send>, lane: LongKind }, Sync { reason: SyncReason, background: bool } }`.
  - `JobRunner::post(&self, w: Work)`; `background_sync(&self, reason) -> bool` (false when
    deduplicated); `is_idle(&self) -> bool`; `close(&self) -> Vec<JobView>` (stops taking work,
    returns the queued jobs it dropped).
  - `Shared::start_job(self: &Arc<Self>, request_id: &str, session: Option<&str>, kind: JobKind, lane: LongKind, run) -> Result<Value, RpcError>`:
    replay (in-memory log, then `JobStore::by_request` → same `JobStarted`), record, post, answer.
  - `syncjob::run_sync(shared: &Arc<Shared>, job_id: Option<&str>, reason: SyncReason, opts: RunOptions) -> SyncResult` — caller holds the lane long.
  - Core: `twydo_core::sync::report::messages(cfg: &Config, r: &RunReport) -> Messages { problems: Vec<String>, warnings: Vec<String>, notes: Vec<String> }`
    (`problems` = today's `problems()`; `warnings` = the `warning:` lines for `warnings`,
    `unexpected`, `superseded`, without the `tdo: ` prefix; `notes` = the `auto_forced` lines);
    `report::collection_id(cfg, name) -> Result<String, String>` moved from `cli/sync.rs`.
- Test helpers (`tests/common`): `fake_vdirsyncer(dir: &Path, body: &str) -> PathBuf` (copy
  `write_script` from `twydo-core/tests/sync_run.rs`, ETXTBSY probe included);
  `Fixture::with_sync(&self, body: &str) -> Config`; `wait_event(c: &Client, limit: Duration,
  pred: impl Fn(&Event) -> bool) -> Event`.
  - `Publisher::set_sync_state(&self, running: bool, job_id: Option<String>)`: what
    `status.sync.state` and `status.sync.job_id` report.

- [ ] **Step 1: Write the failing tests**

`tests/sync.rs` (fake vdirsyncer bodies print `Syncing twydo/c1` to stderr and exit 0 unless
said otherwise):
- `sync_run_answers_at_once_and_reports_finished`: body sleeps 0.5 s; `sync.run` returns a
  `job_id` in < 200 ms; `sync.started {reason: manual}` comes before `sync.finished {outcome:
  ok}`, which comes before `job.finished {state: finished}`; `job.get {job_id}` equals the
  event's state.
- `edit_during_a_sync_is_queued_then_applied`: body sleeps 1 s; submit during it → `queued`;
  exactly one `op.finished {state: applied}` for it arrives (the runner's own drain reports it
  through `report.drained`; the lane's drain hook finds nothing left).
- `manual_sync_waits_behind_a_running_one`: two `sync.run` back to back → both finish `ok`, the
  second `sync.started` after the first `sync.finished`.
- `sync_problem_lines_match_tdo_sync`: body exits 1 with `error: synthetic failure` → outcome
  `failed`, `problems` equals what `tdo sync` printed for the same report before this change
  (assert the literal line from `crates/tdo/tests/sync_e2e.rs` for that case).
- `another_runner_holding_run_lock_is_skipped`: take `sync/run.lock` with `LockFile::take` on the
  test thread; a background `Work::Sync` (via `#[doc(hidden)] DaemonHandle::post_sync_for_test`)
  → `sync.finished {outcome: skipped}` and no error in the log; a manual `sync.run` waits for the
  lock and runs.
`tests/jobs.rs`:
- `resent_sync_run_returns_the_same_job`: same `request_id` twice → same `job_id`; one
  `sync.started`.
- `job_get_by_request_id_after_a_restart`: run a sync job to the end, restart the daemon on the
  same fixture → `job.get {request_id}` is `finished` with the same `job_id`.
- `unfinished_records_become_interrupted_at_startup`: write a `running` record into
  `<state>/daemon/jobs/` (as a killed daemon leaves it), start the daemon → `job.get` says
  `interrupted`; resending its `request_id` to `sync.run` returns that `job_id` and starts no sync.
  (The kill -9 version is Task 6's, once startup reaps the orphaned vdirsyncer.)
- `job_get_unknown_is_not_found`: `job.get {job_id: "job-x"}` → kind `not_found`.

In `twydo-core`, `tests/sync_output.rs` (or a new `tests/sync_report.rs`):
`messages_keep_the_tdo_sync_wording`: the four cases `cli/sync.rs` covers (missing list with the
`--force-delete` hint, collection error, unexpected path, auto-forced list) give today's strings.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon --test sync --test jobs; cargo test -p twydo-core --test sync_output messages_keep`
Expected: FAIL (methods `sync.run` and `job.get` not found; `sync::report` missing).

- [ ] **Step 3: Move the wording to `twydo_core::sync::report`**

Move `problems`, `describe`, `list_names`, `name_of`, `collection_id` and the warning and note
lines; `cli/sync.rs` prints `messages(...)` with its `tdo: ` / `tdo: warning: ` prefixes. Run
`cargo test -p tdo --test sync_e2e` — unchanged output.

- [ ] **Step 4: Implement the job store, the runner and `sync.run`**

- `JobStore` writes records with `fsio`'s atomic write; `Shared` opens it at startup, calls
  `interrupt_unfinished` and `prune(now - retention_days)` before the recovery step.
- `JobRunner`: one thread started at the end of startup (after ready), a `Mutex<VecDeque<Work>>`
  and a `Condvar`. For a `Work::Job`: set `Running`, take `lane.long(lane)`, run, set
  `Finished`/`Failed`, drop the guard (the drain hook runs), emit `job.finished`. For a
  `Work::Sync`: the background pre-check (switch, backoff), then `lane.long(Sync)` and `run_sync`.
- `run_sync`: `publisher.set_sync_state(true, job_id)` + `refresh_status`; emit `sync.started`;
  `sync::run(cfg, &opts, Utc::now())` (`Err` → outcome `failed` with the error as a problem);
  `writes::finish_drain(shared, report.drained)`; `messages`; outcome `stopped` if
  `report.stopped`, `skipped` if `report.skipped`, `failed` if problems, else `ok`. After the
  caller drops the lane: `publisher.reload(Cause::Sync, vec![])`, `set_sync_state(false, None)`,
  `refresh_status`, emit `sync.finished`. Give `run_sync` a closure parameter or split it in
  `begin`/`end` so the end runs after the guard drops; the drain hook must run before the reload.
- `sync.run`: map `force_delete` names with `report::collection_id` (an unknown name → `invalid`
  with the core's message), then `start_job(…, JobKind::Sync, LongKind::Sync, …)` whose body is
  `run_sync(…, SyncReason::Manual, RunOptions { background: false, force_delete })`.
- `job.get`: `JobStore::get`/`by_request`; neither or both given → `invalid`; none found →
  `not_found`.
- Add `SyncView.job_id: Option<String>` (`skip_serializing_if`); `build_status` reports
  `state: running` and the id from `set_sync_state`.

- [ ] **Step 5: Add the schema rows, regenerate, run everything**

Rows: `sync.run` (`SyncRunParams` → `JobStarted`), `job.get` (`JobGetParams` → `JobView`).
Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): run syncs as jobs on the vdir lane with job.get"
```

---

### Task 4: `sync.stop` and `sync.set_auto`

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/sync.rs`, `server/syncjob.rs`, `server/dispatch.rs`, `schema.rs`
- Test: `crates/twydo-daemon/tests/sync.rs`

**Interfaces:**
- Consumes: `JobRunner`, `run_sync` (Task 3).
- Produces: `SyncStopResult { stopping: bool }` (false when nothing ran);
  `SyncSetAutoParams { request_id: String, on: bool }` → `SyncSetAutoResult { auto: bool, message: String }`
  (`message` = `cli::sync::switch_message(on)`, moved to `sync::report::switch_message`);
  `Shared::after_switch_on(self: &Arc<Self>)` (Task 5 fills it; here a no-op hook).

- [ ] **Step 1: Write the failing tests**

- `sync_stop_ends_a_running_sync`: body `sleep 30`; `sync.run`, then `sync.stop` after
  `sync.started` → `{stopping: true}`; `sync.finished {outcome: stopped}` within 5 s.
- `stop_with_nothing_running_does_not_block_the_next_sync` (Review Focus 3): `sync.stop` with no
  sync → `{stopping: false}`; an immediate `sync.run` finishes `ok` (not `stopped`).
- `stop_request_is_withdrawn_after_the_stopped_sync`: after the stopped sync, `sync/stop` does not
  exist in the state directory and the next `sync.run` finishes `ok`.
- `set_auto_persists_the_switch`: `sync.set_auto {on: false}` → `status.sync.auto == false`,
  `<state>/sync/off` exists, message `switch_message(false)`; resending the same `request_id`
  returns the same result; `{on: true}` removes the file.
- `writes_during_a_stopped_sync_drain_after_it`: queue an edit during the sync, stop it → the edit
  is applied after `sync.finished`.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon --test sync stop set_auto writes_during`
Expected: FAIL (method not found).

- [ ] **Step 3: Implement**

- `sync.stop`: no `request_id` (idempotent, writes nothing a client could lose). If no sync runs
  in this daemon (`set_sync_state` says not running), answer
  `{stopping: false}` without touching `sync/stop`. Else `running::request_stop(cfg)`, remember
  `stop_requested = true` in the runner; `run_sync` calls `running::clear_stop(cfg)` after the
  runner returns when that flag is set, then clears it.
- `sync.set_auto`: replayed through the request log; `switch::set(cfg, !on)`;
  `refresh_status`; when `on`, call `after_switch_on`.

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): stop a running sync and switch automatic sync"
```

---

### Task 5: The scheduler

**Files:**
- Create: `crates/twydo-daemon/src/server/schedule.rs`, `crates/twydo-daemon/tests/schedule.rs`
- Modify: `server/mod.rs` (options, start, `after_switch_on`, `poke`), `server/writes.rs`
  (`publish_applied` pokes), `tests/common/mod.rs`

**Interfaces:**
- Produces:
  - `pub trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }`, `SystemClock`.
  - `pub struct SchedState { pub startup_pending: bool, pub next_periodic: DateTime<Utc> }`.
  - `pub enum Next { Sync(SyncReason), Wait(DateTime<Utc>) }`.
  - `pub fn next(s: &SchedState, batch: Due, now: DateTime<Utc>) -> Next` — pure:
    startup pending → `Sync(Startup)`; `batch == Now` → `Sync(Batch)`; `now >= next_periodic` →
    `Sync(Periodic)`; else `Wait(min(next_periodic, batch At))`.
  - `Scheduler::start(shared: Weak<Shared>, clock: Arc<dyn Clock>, periodic: Duration) -> Scheduler`;
    `poke(&self)`; `sync_now(&self, reason: SyncReason)`; `stop(&self)`.
  - `DaemonOptions { schedule: bool (default true), periodic: Duration (60 s), clock: Arc<dyn Clock> }`;
    `#[doc(hidden)] DaemonHandle::poke_scheduler(&self)`.
  - `Shared::poke_scheduler(&self)`, `Shared::sync_now(&self, reason)`.
  - `tests/common`: `TestDaemon` sets `opts.schedule = false`; `TestDaemon::start_scheduled(fx, cfg,
    clock: Arc<FakeClock>)` turns it on.

- [ ] **Step 1: Write the failing tests**

Unit tests in `schedule.rs` for `next`: startup first; `Due::Now` before periodic; periodic when
due; `Wait` is the earlier of `next_periodic` and the batch deadline; `Due::Nothing` waits for
periodic.

`tests/schedule.rs` (a `FakeClock { now: Mutex<DateTime<Utc>> }` with `advance`; scheduler on;
`periodic = 60 s`; a fake vdirsyncer that appends a line to `calls` per `sync`; after each
`advance`, `poke_scheduler()` and wait for `sync.finished` or 300 ms of quiet):
- `startup_sync_runs_once`: one `sync.started {reason: startup}` after ready.
- `periodic_sync_every_interval`: advance 61 s → one `periodic`; advance 30 s → none.
- `clock_jump_runs_one_periodic_sync` (Review Focus 2): advance 2 h → exactly one sync.
- `batch_size_reached_syncs_after_the_quiet_pause`: `batch_size = 2`; two edits; advance 4 s →
  one `sync.started {reason: batch}`.
- `max_wait_syncs_a_single_edit`: one edit; advance `max_wait_secs + 1` → one `batch` sync.
- `switched_off_runs_no_background_sync`: `sync.set_auto {on: false}`; advance 2 min → no sync;
  `sync.run` still runs.
- `switch_on_with_unsynced_edits_syncs_at_once`: off, edit, on → `sync.started {reason: switch_on}`.
- `failing_batch_sync_backs_off_instead_of_looping` (Review Focus 1): body exits 1; one edit due
  now → one sync, `status.sync.next_attempt` in the future; advance 10 s → no second sync.
- `busy_drain_is_retried_by_the_scheduler`: hold `VdirLock::exclusive` from the test thread while a
  queued item drains (the drain gives up as `Busy`), release, advance 5 s → the item is applied.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon schedule`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement**

The loop: compute `next(&state, batch::due(cfg, now)?, now)` (an error from `due` logs and waits
5 s). On `Sync(r)`: `runner.background_sync(r)`; set `startup_pending = false`; for every
background sync posted, `next_periodic = now + periodic`. On `Wait(t)`: wait on the condvar until
poked or `min(t - now, 5 s)`. Each wake also calls `request_drain` when the queue has items and
the lane is free (plan-1 deferred: a `Busy` drain had no later trigger). `poke` is called after
every applied or drained write (`publish_applied`) and at the end of every sync. `sync_now(r)`
posts at once. `after_switch_on` calls `sync_now(SwitchOn)` when `batch::due` is not `Nothing`.
With `schedule = false` no thread starts and `poke`/`sync_now` do nothing. `teardown` stops the
scheduler before closing the runner.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(daemon): schedule startup, periodic and batched syncs"
```

---

### Task 6: Orphaned vdirsyncer at startup and shutdown during a job

**Files:**
- Modify: `crates/twydo-core/src/sync/running.rs`, `server/mod.rs` (startup step, teardown,
  `daemon.shutdown`), `server/dispatch.rs`, `server/jobs.rs`
- Test: `crates/twydo-core/tests/sync_run.rs`, `crates/twydo-daemon/tests/lifecycle.rs`,
  `crates/tdo/tests/daemon_process.rs`

**Interfaces:**
- Produces: `running::reap_orphan(cfg: &Config) -> io::Result<Reaped>` with
  `enum Reaped { Nothing, Stale, Killed, LiveRunner }`: no record → `Nothing`; runner alive →
  `LiveRunner` (another process's sync; left alone); runner gone, child alive → `request_stop`
  (the orphan path), then `clear_stop`, → `Killed`; both gone → record removed → `Stale`.
  `DaemonOptions::shutdown_grace: Duration` (10 s).

- [ ] **Step 1: Write the failing tests**

Core: `reap_orphan_kills_a_child_whose_runner_is_gone` (start `sh -c 'sleep 30'` in its own
group, write `running.json` naming a dead runner pid and that child → `Killed`, the child is
gone, `sync/stop` does not exist); `reap_orphan_leaves_a_live_runner` (runner = this test
process → `LiveRunner`, record kept); `reap_orphan_removes_a_stale_record`.

Daemon (`tests/lifecycle.rs`):
- `startup_reaps_an_orphaned_vdirsyncer`: as the core test, then start the daemon → ready within
  5 s, the child is gone, the vdir lock is free (`VdirLock::exclusive` succeeds).
- `sigterm_during_a_sync_loses_no_edit` (spec §13): body `sleep 30`; `sync.run`; queue an edit;
  `shutdown_grace = 200ms`; `handle.shutdown()` returns within 5 s; `sync.finished {outcome:
  stopped}` was sent; the queue item is still on disk; a new daemon applies it once.
- `queued_jobs_are_interrupted_at_shutdown`: a running sync plus a queued `sync.run` → after
  shutdown the queued job's record is `interrupted`.
- `shutdown_when_idle_waits_for_jobs`: body `sleep 1`; `sync.run`, then `daemon.shutdown
  {when_idle: true}` → replies at once; a `sync.run` after it is `unavailable`; the daemon stops
  only after `sync.finished {outcome: ok}`.

Process (`crates/tdo/tests/daemon_process.rs`, the real binary with a fake vdirsyncer config):
- `job_cut_short_by_kill_is_interrupted` (Review Focus 4): body `sleep 30`; `sync.run` through
  `twydo_daemon::client::Client`; `kill -9` the daemon after `sync.started`; start it again →
  ready within 5 s (the orphaned vdirsyncer is reaped), `job.get {request_id}` is
  `interrupted`, resending the same `request_id` returns the same `job_id`, and no
  `sync.started` follows for it.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-core --test sync_run reap_orphan; cargo test -p twydo-daemon --test lifecycle`
Expected: FAIL.

- [ ] **Step 3: Implement**

- Startup: a step before recovery calls `reap_orphan` (`[sync]` configured only); `Killed` logs
  `"stopped a vdirsyncer left by an earlier run"`; `LiveRunner` logs and continues (its locks
  make the daemon's own writes wait or queue).
- Teardown, before `close_connections`: `runner.close()` (queued jobs → `interrupted`, with
  `job.finished`); if a sync runs, wait `shutdown_grace` for it, then `request_stop` and wait up
  to 10 s more; a list job is waited for. Then the existing teardown.
- `daemon.shutdown {when_idle: true}`: set `closing` (new jobs and `sync.run` → `unavailable`
  "the daemon is shutting down"; the scheduler posts nothing), reply, and a thread waits for
  `runner.is_idle()` before `request_stop`. `when_idle: false` keeps plan 1's behaviour.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(daemon): reap an orphaned vdirsyncer and stop cleanly during a sync"
```

---

### Task 7: `conflicts.list` and `conflicts.resolve`

**Files:**
- Create: `crates/twydo-daemon/src/protocol/admin.rs`, `crates/twydo-daemon/src/server/admin.rs`,
  `crates/twydo-daemon/tests/admin.rs`
- Modify: `crates/twydo-core/src/conflicts.rs`, `crates/twydo-core/src/sync/resolve.rs`,
  `server/lane.rs`, `server/dispatch.rs`, `schema.rs`
- Test: `crates/twydo-core/tests/conflicts.rs`, `crates/twydo-core/tests/resolve.rs`

**Interfaces:**
- Produces (core): `ResolvedBy { request: RequestRef, resolution: Resolution, op_id: Option<String> }`
  (serde); `Conflict.resolved_by: Option<ResolvedBy>`; `ConflictStore::by_request(&self, request_id: &str) -> io::Result<Option<Conflict>>`;
  `resolve_with(cfg, id, keep, now, request: Option<RequestRef>) -> Result<Resolution, ResolveError>`
  (`resolve` calls it with `None`; the `Written` path calls `Store::set_request` before `restore`).
  `Resolution` derives `Serialize, Deserialize` (`snake_case`; `Written(String)` serializes as
  `{"written": "<op id>"}`).
- Produces (daemon): `Lane::write_waiting(&self) -> WriteGuard<'_>` (waits while anyone holds the
  lane). `ConflictView { id, uid, collection, path, title: Option<String>, kind: WireConflictKind, held: bool, state: WireConflictState, choice: Option<WireSide>, detected }`
  — wire enums open, mirroring `ConflictKind`, `ConflictState`, `Side` in snake_case; `title` is
  the `SUMMARY` that `cli/conflicts.rs::title` shows (move that helper to `conflicts.rs`).
  `ConflictsResolveParams { request_id, id, keep: WireKeep (local|remote|current) }` →
  `ResolveResult { resolution: WireResolution (open: recorded|already_in_place|written|kept_current), op_id: Option<String>, message: String }`
  with `tdo conflicts resolve`'s wording.

- [ ] **Step 1: Write the failing tests**

Core: `resolution_records_its_request` (each of the four outcomes stores `resolved_by` with the
request; `by_request` finds it); `old_conflict_files_still_load` (a record without
`resolved_by`).

Daemon (`tests/admin.rs`; seed conflicts with `ConflictStore::record` on the fixture's state):
- `conflicts_list_shows_unresolved_with_titles`.
- `resolve_written_publishes_and_records`: non-held conflict, keep remote → `written` with an
  `op_id`, a `data.changed`, and the conflict leaves `conflicts.list`.
- `resolve_recorded_starts_a_sync`: held conflict → `recorded`; with the scheduler on and auto on,
  a `sync.started {reason: resolve}` follows.
- `resend_after_restart_replays_the_resolution`: resolve, restart, resend the same
  `request_id` → the same result, no second journal entry.
- `resolve_during_a_sync_waits_for_it`: body `sleep 1`; resolve during the sync → answered after
  `sync.finished`, `written`.
- `resolve_unknown_conflict_is_not_found`.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-core --test conflicts --test resolve; cargo test -p twydo-daemon --test admin conflicts resolve`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `conflicts.resolve`: replay (log, then `ConflictStore::by_request` → result rebuilt from
  `resolved_by`); take `lane.write_waiting()`, then the commit gate; `resolve_with` with the
  connection's session and the `request_id`. Then `publisher.reload(Cause::Op, vec![uid])`,
  `refresh_status`, and `sync_now(Resolve)` for `Recorded` (when auto is on), else `poke`.
  `ResolveError::NotFound` → `not_found`; `NotUnresolved`, `Held`, `Changed` → `invalid` with the
  core message; I/O → `internal`.
- `conflicts.list`: unresolved only, as `tdo conflicts` (archived-list conflicts excluded, as
  `sync::summary` does).

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): list and resolve conflicts with replayable resolutions"
```

---

### Task 8: `journal.list`, `journal.recover`, `journal.revert`, `journal.abandon`

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs` (`abandon`), `protocol/admin.rs`, `server/admin.rs`,
  `server/writes.rs`, `server/dispatch.rs`, `schema.rs`
- Test: `crates/twydo-core/tests/journal.rs`, `crates/twydo-daemon/tests/admin.rs`

**Interfaces:**
- Produces: `JournalEntryView { id, created, kind: String, stage: WireStage (open: prepared|applied|abandoned), uids: Vec<String>, synced: Option<DateTime<Utc>>, request_id: Option<String> }`;
  `journal.list {}` → `Vec<JournalEntryView>` (oldest first, as `tdo op journal`);
  `journal.recover {}` → `RecoverResult { resumed: Vec<String>, stuck: Vec<StuckView { id, reason }> }`;
  `JournalRevertParams { session: String, request_id: String, op_id: String }` → `WriteResult`;
  `JournalAbandonParams { request_id: String, op_id: String }` → `AbandonResult { abandoned: bool }`.
  Core: `Store::abandon` returns `Ok(false)` for an entry already `Abandoned`, `Ok(true)` when it
  abandoned one (callers in `cli/op.rs` print `abandoned <id>` either way).
  `writes::run_store_write(shared, session: &str, request_id: &str, f: impl FnOnce(&mut Store, DateTime<Utc>) -> Result<Submitted, WriteError>) -> Result<WriteResult, RpcError>`
  — the lane-or-queue part of `run_submit`, extracted so `op.submit` and `journal.revert` share it.

- [ ] **Step 1: Write the failing tests**

Core: `abandon_twice_is_not_an_error`.

Daemon:
- `journal_list_matches_the_store`: after two edits, two entries with stage `applied`, their
  uids and `request_id`s.
- `journal_recover_resumes_and_reports_stuck`: leave a `Prepared` entry whose writes cannot be
  finished → `stuck` names it, `status.journal.stuck` too; a `data.changed {cause: recovery}` when
  something resumed.
- `revert_by_id_from_another_session_keeps_stacks`: session A edits; session B reverts A's entry
  by id → applied; A's `op.undo` still undoes A's own edit (D7: the revert is not on A's stack).
- `revert_during_a_sync_is_queued`: hold the lane long → `journal.revert` answers `queued`.
- `abandon_is_idempotent_and_clears_stuck`: abandon a stuck entry → `{abandoned: true}`,
  `status.journal.stuck` empty; a new `request_id` → `{abandoned: false}`.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-core --test journal abandon_twice; cargo test -p twydo-daemon --test admin journal revert abandon`
Expected: FAIL.

- [ ] **Step 3: Implement**

`journal.recover` and `journal.abandon` take `lane.write_waiting()` and the commit gate;
recover runs `Store::open_with` (its report), then `publisher.set_stuck`, `reload(Cause::Recovery)`,
`refresh_status`. `journal.revert` goes through `replayed` and `run_store_write` with a fresh
`Store::without_recovery` (not the session's store), `set_request(session, request_id)` and
`store.undo(op_id, now)`; message `"undone: <title>"` from `edit::describe`.

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): journal list, recover, revert and abandon"
```

---

### Task 9: Live `config.toml` and `input.parse`

**Files:**
- Create: `crates/twydo-daemon/src/server/config.rs`, `crates/twydo-daemon/tests/config.rs`
- Modify: `server/publish.rs` (`cfg` behind a mutex; `set_config`), `server/watch.rs`
  (`start_file`), `server/mod.rs` (startup step), `protocol/methods.rs`, `protocol/admin.rs`,
  `server/admin.rs`, `server/dispatch.rs`, `schema.rs`

**Interfaces:**
- Produces: `watch::start_file(path: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher>`
  (watches the parent directory non-recursively; fires for events naming `path`'s file name);
  `Publisher::set_config(&self, cfg: Config)`; `Publisher::config(&self) -> Config`;
  `StatusView.config: ConfigStatus { error: Option<String>, restart_needed: Vec<String> }`
  (field names from `vdir`, `state_dir`, `writes`, `sync`).
  `InputParseParams { field: InputField (due|start|repeat), text: String, today: Option<NaiveDate> }`
  → `InputParsed { due: Option<Due>, repeat: Option<RepeatSpec>, cleared: bool, description: String }`;
  a parse error is `invalid` with the parser's message. `today` defaults to today in
  `Config::tz()` of the live configuration; repeat uses its `work_days`.

- [ ] **Step 1: Write the failing tests**

`tests/config.rs` (a fixture whose instance config is a real file in the temp dir):
- `list_preference_change_publishes_cause_config`: change a `[lists.<id>]` preference → within
  2 s `data.changed {cause: config}` and `snapshot.config` has it.
- `binding_change_needs_a_restart`: change `[writes] mode` → `status.config.restart_needed ==
  ["writes"]`; the snapshot keeps the old mode.
- `broken_config_keeps_the_last_good_one` (Review Focus 5): write `vdir = ` (invalid) →
  `status.config.error` is `Some`, the snapshot unchanged; write a valid file with a new
  `week_start` → error `None`, new value published.
- `config_saved_by_rename_is_seen`: write to `config.toml.tmp` and rename over → reloaded.
- `vdir_flag_override_is_kept`: a daemon started with `--vdir` (instance `vdir: Some`) reloads a
  config naming another vdir without `restart_needed` for `vdir`.

`tests/admin.rs`:
- `input_parse_due_and_repeat`: `{field: due, text: "fri 5pm", today: 2026-11-10}` → `due` is
  `Time(2026-11-13T17:00)` and `description` equals `dateinput::describe`; `{field: repeat, text:
  "2nd tue"}` → `repeat.rrule` `FREQ=MONTHLY;BYDAY=2TU` (the value `repeatinput` gives today);
  `{text: ""}` → `cleared: true`; `{text: "9"}` → `invalid`.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon --test config; cargo test -p twydo-daemon --test admin input_parse`
Expected: FAIL.

- [ ] **Step 3: Implement**

`config::changed(shared)`: read the bytes; unchanged since the last read → nothing. `Config::load`
(error → `ConfigStatus.error`, keep the current config); apply the instance's `--vdir` override
as `resolve_config` does; compare the binding fields with the daemon's binding config (vdir and
state dir compared canonical/absolute, as `term/preferences.rs` does) → `restart_needed`; build
the new live config = loaded with the binding fields of the old; if it differs,
`publisher.set_config` and `reload(Cause::Config)`; `refresh_status`. Started as a startup step
after the vdir watcher; a missing config directory skips the watch with a log line.

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): reload config.toml live and parse typed dates and repeats"
```

---

### Task 10: List jobs: create, update, archive, delete, reconcile, export

**Files:**
- Create: `crates/twydo-daemon/src/protocol/lists.rs`, `crates/twydo-daemon/src/server/lists.rs`,
  `crates/twydo-daemon/tests/lists.rs`
- Modify: `crates/twydo-core/src/store/queue.rs` (`push` without the fence),
  `protocol/jobs.rs` (`JobResult` variants), `server/dispatch.rs`, `schema.rs`
- Test: `crates/twydo-core/tests/store_queue.rs` (fence expectations)

**Interfaces:**
- Produces: `WireListState { id, name, color: Option<String> ("#rrggbb"), archived }` with
  `From<ListState>` / `TryFrom<WireListState>`;
  `ListCreateParams { request_id, id: String, name, color: String }`;
  `ListUpdateParams { request_id, id, name, color: Option<String>, expected: WireListState }`;
  `ListArchiveParams { request_id, expected: WireListState }` (for `lists.archive` and `lists.unarchive`);
  `ListPreviewDeleteParams { request_id, id }`; `ListDeleteParams { request_id, preview: WireDeletePreview }`;
  `WireDeletePreview { backup, collection, name, resources: usize, local_files: usize }`;
  `ListReconcileParams { request_id, operation: String, keep_server: bool }`;
  `ListExportParams { request_id, backup: String, destination: PathBuf }`.
  All answer `JobStarted`. `JobResult` gains `ListSaved { operation: String, state: WireListState }`,
  `DeletePreview(WireDeletePreview)`, `Exported { backup: String, destination: PathBuf }`.
  A `ListError::Refused(m)` fails the job with kind `blocked` and `m`; `ListError::Io` with `internal`.

- [ ] **Step 1: Write the failing tests**

`tests/lists.rs` (the list adapter is a fake helper: reuse `twydo-core/tests/lists_helper.rs`'s
synthetic helper setup, `[sync] list_python` pointing at it, `writes.mode = test`, and the
fixture matrix rows those tests use):
- `create_list_job_finishes_with_the_state`: `lists.create` → `JobStarted`; `job.finished` has
  `list_saved` with the name and colour; a `data.changed {cause: list}` shows the collection.
- `update_with_a_stale_expected_state_fails`: `expected` with another name → job `failed`, kind
  `blocked`, message `"list changed; reload and try again"`.
- `archive_and_unarchive_write_the_config`: `lists.archive` → state `archived: true` and a
  `data.changed {cause: config}` (Task 9's watcher sees the file the job wrote).
- `delete_keeps_todays_refusal`: `lists.preview_delete` → `delete_preview` result;
  `lists.delete` with it → `failed` with the core's "List deletion is unavailable…" message.
- `edit_during_a_list_job_is_queued_and_fenced_at_drain`: a helper that sleeps 1 s while it
  edits list `c1`; submit an edit of a task in `c1` during the job → `queued`; after the job,
  `op.finished` `applied` (the job ended) — and with a helper that leaves the operation active
  (a reconcile needed), `refused` with the fence's message.
- `export_writes_the_backup_file`.
- `resent_list_job_returns_the_same_job`.

Core: in `store_queue.rs`, the test that expected `Queue::push` to refuse during an active list
operation now expects the push to succeed and the drain to refuse with the same message.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon --test lists; cargo test -p twydo-core --test store_queue`
Expected: FAIL.

- [ ] **Step 3: Implement**

Each method is `start_job(…, kind, LongKind::List, body)`; the body opens
`ListService::open_with(&binding_cfg, config_path, matrix)` where `config_path` is the
instance's config file when it exists, and calls the same service method as
`tdo/src/term/lists.rs::perform_list_action` does for that action. After the body:
`reload(Cause::List)`. Remove the `ensure_idle` call from `Queue::push`.

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): run list create, update, archive and delete as jobs"
```

---

### Task 11: Retired lists: forget, recover, backups

**Files:**
- Modify: `protocol/lists.rs`, `protocol/jobs.rs`, `server/lists.rs`, `server/dispatch.rs`, `schema.rs`
- Test: `crates/twydo-daemon/tests/lists.rs`

**Interfaces:**
- Produces: `lists.preview_forget { name }` → `ForgetPreview { collection, name, tasks: usize, pending: usize }`;
  `ListForgetParams { request_id, name: String, collection: String }` (the job re-previews and
  fails `blocked` "list changed; reload and try again" when `collection` differs);
  `ListRecoverParams { session: String, request_id, backup: String, to: String }`;
  `lists.backups {}` → `Backups { retired: Vec<RetiredBackupView { id, collection, name, created, tasks, pending, retired: bool }>, deleted: Vec<DeletedBackupView { id, collection, name, created }> }`;
  `lists.backup_preview { id }` → `WireSnapshot` of `retired::snapshot` (revision 0).
  `JobResult` gains `Forgot { backup: String, name: String, message: String, sync: Option<SyncResult> }`
  and `Recovered { op_id: Option<String>, applied: bool, message: String }`.

- [ ] **Step 1: Write the failing tests**

- `forget_preserves_and_syncs`: `lists.forget` → `forgot` with today's message
  (`preserved "<name>" as <id>; recover with :recover <id> <destination-list>`) and a `sync`
  result; one `sync.started {reason: list}`; the list leaves the snapshot.
- `forget_with_a_changed_list_fails`.
- `recover_into_a_list_applies_once`: `lists.recover` → `recovered {applied: true}` with an
  `op_id` and the TUI's message; resending the `request_id` → the same job; a new request for the
  same backup → `applied: false`, message `"already recovered; backup retained"`.
- `recover_waits_behind_a_sync_instead_of_failing` (spec §7.4): body `sleep 1`; `lists.recover`
  during it → finishes `recovered` after `sync.finished`.
- `backups_list_both_kinds` and `backup_preview_shows_the_tasks`.

- [ ] **Step 2: Run them**

Run: `cargo test -p twydo-daemon --test lists forget recover backups backup_preview`
Expected: FAIL.

- [ ] **Step 3: Implement**

`lists.forget`: body runs `retired::forget`, then, holding the same lane, `run_sync(…, List,
RunOptions::default())` (a manual sync, as the TUI does), and folds its outcome into
`Forgot.message` the way `term.rs::start_forget` does. `lists.recover`: body runs
`Store::without_recovery` with `set_request(session, request_id)` and `recover_list` against
the current published snapshot; then `reload(Cause::List)` and `poke_scheduler`.

- [ ] **Step 4: Regenerate the schema, run everything**

Run: `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates schema
git commit -m "feat(daemon): forget and recover lists as jobs and list their backups"
```

---

### Task 12: Record progress

**Files:**
- Modify: `AGENTS.md` (progress note), `docs/superpowers/specs/2026-11-09-daemon-design.md` only
  where this plan decided a builder's call that changes wording there (`sync.run` takes a
  `request_id`; no `job.progress` yet)

- [ ] **Step 1: Update the progress note**

AGENTS.md: daemon plan 2 done on its branch; what the daemon serves now; next is plan 3 (clients
switch; its deferred list in plan 1's review notes); clients still run their own syncs until then.

- [ ] **Step 2: Check links and run the full suite**

Run: `python3 tools/check_links.py && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md docs
git commit -m "docs: record daemon plan 2 progress"
```
