# Issue #10: Stop a Stuck Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sync stuck in vdirsyncer (e.g. `pass` waiting on pinentry) no longer costs the user
an edit: the TUI offers to stop the sync and then saves.

**Architecture:** While vdirsyncer runs, the runner keeps `sync/running.json` (its own pid,
the child's process group, the start time). Any `tdo` can ask it to stop by creating
`sync/stop`; the runner polls for that file the way it polls for its timeout, kills the group
and records the run as `sync stopped` (a transient failure, so it backs off). If the runner
itself is gone (Ctrl-C'd `tdo sync`, orphaned vdirsyncer), the requester kills the group
directly. The TUI waits briefly for the lock, then asks `stop it and save? (y/n)`. Separately,
`metasync` no longer runs after a failed `sync`, so a stuck run blocks once, not twice.

**Tech Stack:** Rust 2024 workspace (`twydo-core`, `tdo`), serde_json, ratatui; process
control through `kill(1)` as today (no libc dependency).

**Spec:** the design agreed in chat on 2026-11-03 (summarised here; it is the source of truth for
this plan), issue [#10](https://github.com/bwilczynski/twydo/issues/10), and spec §5 ("A sync
failure must not disable reminders from the local data"; serialize runner invocations and
coordinate them with local edits).

Agreed design:

1. A save that finds a sync running waits up to about 2 s; if the lock frees, it saves silently.
2. Still running: the status line asks `sync running for 2m 14s — stop it and save? (y/n)`. The
   TUI never mentions passphrases (it does not know why the sync is slow).
   - `y`: stop the sync (whoever started it: the TUI, the timer, a `tdo sync`), then save; the
     message says the sync was stopped. The stopped run is a failed sync: it backs off, its lists
     show `error`, the edit stays pending until the next good sync.
   - `n` or `Esc`: nothing is written; an open form stays open with its values; a one-key action
     leaves the task as it was.
3. Skip `metasync` when `sync` failed (CLI, timer and TUI alike).
4. Out of scope: detecting `pass`/gpg, a `tdo sync --stop` command, queueing edits.

## Global Constraints

- No personal data in the repository (AGENTS.md); tests use fake `sh` vdirsyncer scripts.
- No new dependencies. Signals go through `kill(1)` (`/bin/kill`, then `kill`), like
  `kill_tree` in `crates/twydo-core/src/sync/run.rs`.
- Lock order stays: run lock, vdir lock, conflicts lock. The stop request takes no lock.
- Exact copy:
  - prompt: `sync running for {elapsed} — stop it and save? (y/n)`
  - elapsed: `45s`, `2m 14s`, `1h 3m` (seconds under a minute; minutes and seconds under an
    hour; hours and minutes above)
  - after `n`/`Esc`: `not saved; the sync is still running`
  - after `y` and a successful save: `<the usual success message> · sync stopped`
  - after `y` when the lock is still held after the stop wait: `not saved; the sync did not stop`
  - the stopped run's transient error and list errors: `sync stopped`; metasync: `metasync stopped`
  - lock held with no `running.json` (a reader, or a runner between vdirsyncer calls): the
    existing `a sync is running; try again in a moment`
- Docs change in the same commit as the behaviour (`docs/sync.md`, `docs/tui.md`,
  `docs/architecture.md` if it lists state files); `python3 tools/check_links.py` passes.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --check` pass at the end of every task.

## Review Focus

1. **The sync finishes on its own while the prompt is open.** `y` then finds nothing to stop;
   the save must go through with the plain success message (no `· sync stopped`). Test in Task 3.
2. **A stale `sync/stop`** (the TUI asked, but the runner had just finished) must not kill the
   next run. The runner deletes it before each vdirsyncer call. Test in Task 2.
3. **An orphaned vdirsyncer** (runner Ctrl-C'd) holds the lock with nobody polling `sync/stop`;
   the stop must kill the recorded group directly. Test in Task 2.
4. **A stopped run must not rerun** because `sync/requested` exists (the TUI's own after-edit
   trigger creates it), or the user's stop is undone at once. Test in Task 2.
5. **`n` on an edit form whose task changed meanwhile**: goes through `write_finished(Err)`, so
   the existing `refresh_form` rules apply (form rebuilt from the saved version). Test in Task 4.

---

### Task 1: Skip `metasync` after a failed `sync`

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs` (`one_pass`, the `report.metasync_error = …` match)
- Test: `crates/twydo-core/tests/sync_run.rs`
- Docs: `docs/sync.md` (step 4 of "What `tdo sync` does"; the "locked password store" known limit)

**Interfaces:**
- Produces: nothing new; `metasync` runs only when the sync pass is not a transient failure.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn metasync_is_skipped_when_sync_failed() {
    // fake: append "$1" to calls; `sync` prints "error: Connection refused" and exits 1
    // metasync = true
    // assert: r.transient == Some("Connection refused"); calls file == "sync\n";
    //         r.metasync_error == None
}

#[test]
fn metasync_still_runs_after_a_clean_sync() {
    // fake: append "$1" to calls, exit 0; metasync = true
    // assert: calls file == "sync\nmetasync\n"
}
```

- [ ] **Step 2: Run them; the first fails** (`calls` holds `sync\nmetasync\n`)

Run: `cargo test -p twydo-core --test sync_run metasync_`

- [ ] **Step 3: Implement**

Run metasync only when `ran.outcome` is `Exited(_)` **and**
`transient(sync, &ran, &out, &coll_states(&out, &[], &sync.local_storage))` is `None`. Conflicts
and emptied lists are not transient, so metasync still runs after them, as today.

- [ ] **Step 4: Run `cargo test -p twydo-core --test sync_run`**: all pass.

- [ ] **Step 5: Docs.** In `docs/sync.md` step 4: "Then `vdirsyncer metasync <pair>` runs if
  `metasync` is on and the sync did not fail." Adjust the known limit to "each run waits for
  pinentry once".

- [ ] **Step 6: Commit** `fix(sync): skip metasync when the sync failed`

---

### Task 2: The runner records itself and honours a stop request

**Files:**
- Create: `crates/twydo-core/src/sync/running.rs`
- Modify: `crates/twydo-core/src/sync/mod.rs` (`pub mod running;`)
- Modify: `crates/twydo-core/src/sync/run.rs` (`Outcome::Stopped`, the guard, the poll loop,
  `RunReport::stopped`, no rerun after a stop; `kill_tree` uses `running::kill_group`)
- Test: `crates/twydo-core/tests/sync_run.rs`
- Docs: `docs/sync.md` (step 4, "Backoff and timeout", "State files", the Ctrl-C and locked
  password store known limits), `docs/architecture.md` state-file listing if it names `sync/` files

**Interfaces:**
- Produces (in `twydo_core::sync::running`):

```rust
/// `<state>/sync/running.json`, present while a runner has vdirsyncer running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Running {
    pub runner_pid: u32,
    /// The current vdirsyncer child (it leads its own process group); None between calls.
    pub child_pgid: Option<u32>,
    /// When step 4 began (wall clock, not the run's `now`).
    pub started: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResult {
    /// No `running.json`: nothing to stop.
    NothingRunning,
    /// `sync/stop` created; the live runner kills its child and records `sync stopped`.
    Requested,
    /// The runner is gone; its recorded group was killed and `running.json` removed.
    Killed,
}

/// The running sync of `cfg`'s vdir, if any. Creates nothing; `None` without `[sync]` or state.
pub fn running(cfg: &Config) -> io::Result<Option<Running>>;
/// Asks the running sync to stop (see [`StopResult`]). Never waits for it.
pub fn request_stop(cfg: &Config) -> io::Result<StopResult>;
/// `kill -KILL -- -<pgid>` through kill(1); false if no kill(1) succeeded.
pub(crate) fn kill_group(pgid: u32) -> bool;
/// `kill -0 <pid>` through kill(1).
pub(crate) fn alive(pid: u32) -> bool;
```

- Produces (in `run`): `RunReport { pub stopped: bool, .. }`.
- File names: `sync/running.json`, `sync/stop`.

- [ ] **Step 1: Write the failing tests** (each uses `Fx` and a fake that sleeps; run the
  runner in a thread, `wait_for(state/sync/running.json)`)

```rust
#[test]
fn running_is_reported_while_vdirsyncer_runs() {
    // fake: touch started; sleep 1
    // while running: running(&cfg) is Some, runner_pid == std::process::id(),
    //   child_pgid is Some, started within the last 5 s of Utc::now()
    // after join: running(&cfg) == None, file gone
}

#[test]
fn stop_request_kills_the_run_and_backs_off() {
    // fake (wrapper, no exec): sleep 30 & echo $! > pidfile; wait   (timeout_secs default 300)
    // wait until running.json has child_pgid; request_stop(&cfg) == Requested
    // join within 5 s; r.stopped; r.transient == Some("sync stopped");
    // status.failures == 1, last_error == Some("sync stopped"), c1 state Error "sync stopped"
    //   only if the fake reported c1 (do not assert on lists otherwise)
    // grandchild from pidfile is dead within 3 s (same `alive` loop as the timeout test)
    // sync/stop and sync/running.json are gone
}

#[test]
fn stopped_run_does_not_rerun() {
    // fake: echo run >> count; sleep 30
    // while running: create sync/requested, then request_stop
    // after join: count has 1 line
}

#[test]
fn stale_stop_file_does_not_stop_the_next_run() {
    // create state/sync/stop before run; fake: sleep 1; exit 0
    // r.stopped == false, r.transient == None, sync/stop gone
}

#[test]
fn stop_with_a_dead_runner_kills_the_recorded_group() {
    // spawn `sleep 30` with CommandExt::process_group(0): the orphan
    // dead pid: spawn `true`, wait for it, take its id
    // write running.json { runner_pid: dead, child_pgid: Some(orphan.id()), started: now }
    // request_stop(&cfg) == Killed; orphan.try_wait() is Some within 3 s; running.json gone
}

#[test]
fn stop_with_nothing_running() {
    // request_stop on a fresh Fx == NothingRunning; no sync/stop created
}
```

- [ ] **Step 2: Run them; they fail to compile** (`running` module missing).

Run: `cargo test -p twydo-core --test sync_run`

- [ ] **Step 3: Implement `running.rs`**

`request_stop`: no `running.json` → `NothingRunning`. Runner alive (`alive(runner_pid)`) →
create `sync/stop`, `Requested`. Runner dead → `kill_group(child_pgid)` when set, remove
`running.json`, `Killed`. An unreadable `running.json` is an `io::Error` (`InvalidData`).

- [ ] **Step 4: Wire the runner**

- A guard (private struct in `run.rs`) created at the start of step 4, after the shared lock:
  removes a stale `sync/stop`, writes `running.json` with `child_pgid: None`; removes
  `running.json` on drop. `run_command` takes it and sets `child_pgid` after spawn and back to
  `None` after the child is reaped (rewrite the file each time).
- `run_command`'s poll loop: before the deadline check, if `sync/stop` exists: `kill_tree`,
  wait, remove `sync/stop`, `Outcome::Stopped`. Drain with the short grace, as after a timeout.
- Treat `Outcome::Stopped` like `TimedOut` everywhere `TimedOut` is matched (lists that were
  `Ok`/`Conflict` become `Error { message: "sync stopped" }`, the manifest update is
  `uncertain`, no auto-force), with `transient` = `"sync stopped"` and metasync's message
  `"metasync stopped"`.
- `RunReport::stopped = true` when any pass was stopped; `run` breaks out of the rerun loop
  after a stopped pass (leave `sync/requested` in place).
- `kill_tree` calls `running::kill_group(child.id())` then `child.kill()`.

- [ ] **Step 5: Run `cargo test -p twydo-core`**: all pass, the timeout tests included.

- [ ] **Step 6: Docs** (`docs/sync.md`): step 4 says the runner can be stopped from the TUI and
  records `sync stopped`; "Backoff and timeout" lists a stopped run as a transient failure;
  "State files" gains `sync/running.json` (fields) and `sync/stop`; the Ctrl-C known limit says
  the TUI's stop kills an orphaned vdirsyncer; the locked password store limit says saves offer to
  stop the sync. Run `python3 tools/check_links.py`.

- [ ] **Step 7: Commit** `feat(sync): record the running sync and stop it on request`

---

### Task 3: The store waits briefly; the writer reports Busy and can stop and save

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs` (`no_wait: bool` → `lock_wait: Option<Duration>`)
- Modify: `crates/tdo/src/write.rs`
- Test: `crates/tdo/tests/write.rs` (update the 26 `w.run(` assertions to the new error type)

**Interfaces:**
- Consumes: `twydo_core::sync::running::{running, request_stop, StopResult}` (Task 2).
- Produces:

```rust
// twydo_core::store::Store
/// None: block for the lock (CLI). Some(d): retry the exclusive lock every 25 ms for up to
/// `d`, then `WriteError::Busy` (Some(ZERO) is the old no-wait).
pub fn set_lock_wait(&mut self, wait: Option<Duration>);   // replaces set_no_wait

// tdo::write
pub const LOCK_WAIT: Duration = Duration::from_secs(2);
pub const STOP_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The vdir lock stayed held for the whole wait. `running_for`: how long the recorded
    /// sync has run (None: no `running.json`, nothing the TUI can stop).
    Busy { running_for: Option<Duration> },
    Other(String),
}
impl Refused { pub fn message(&self) -> String }  // Busy → "a sync is running; try again in a moment"

impl Writer {
    /// Writes with the session's lock wait (LOCK_WAIT unless set).
    pub fn run(&mut self, snap: &Snapshot, req: &Request, now: DateTime<Utc>) -> Result<String, Refused>;
    /// Asks the running sync to stop, then writes waiting up to STOP_WAIT (or the set stop wait).
    /// Ok message gains " · sync stopped" only when the stop did something (Requested/Killed).
    /// Still busy → Refused::Other("not saved; the sync did not stop").
    pub fn stop_and_run(&mut self, snap: &Snapshot, req: &Request, now: DateTime<Utc>) -> Result<String, Refused>;
    /// For tests.
    pub fn set_waits(&mut self, lock: Duration, stop: Duration);
}
```

- [ ] **Step 1: Write the failing tests** (`crates/tdo/tests/write.rs`)

```rust
#[test]
fn write_waits_briefly_for_a_short_sync() {
    // hold VdirLock::shared, release it from a thread after 300 ms
    // w.run(complete_a) == Ok("done") ; elapsed < 2 s
}

#[test]
fn write_during_a_long_sync_is_busy_after_the_wait() {
    // replaces write_during_sync_is_refused_without_waiting
    // w.set_waits(300 ms, 1 s); hold the shared lock; write running.json (runner_pid = own pid,
    //   started = Utc::now() - 134 s)
    // w.run == Err(Refused::Busy { running_for: Some(d) }) with d within 134..140 s;
    // 300 ms <= elapsed < 2 s; file unchanged; Undo == Err(Refused::Other("nothing to undo"))
}

#[test]
fn busy_without_a_recorded_sync_has_no_duration() {
    // shared lock held, no running.json → Err(Refused::Busy { running_for: None })
}

#[test]
fn stop_and_run_when_the_sync_already_finished_saves_plainly() {
    // no lock, no running.json → stop_and_run == Ok("done")   (no " · sync stopped")
}

#[test]
fn stop_and_run_kills_an_orphaned_sync_and_saves() {
    // orphan `sh -c 'exec sleep 30' 0<vdir.lock` in its own process group, holding the shared
    //   lock through stdin (use VdirLock::shared_for_child for its stdin);
    // running.json with a dead runner_pid and child_pgid = orphan
    // w.set_waits(300 ms, 3 s); stop_and_run == Ok("done · sync stopped"); task file completed
}

#[test]
fn stop_and_run_gives_up_when_the_lock_stays_held() {
    // shared lock held in-process; running.json with runner_pid = own pid (alive, never polls)
    // w.set_waits(100 ms, 300 ms); stop_and_run == Err(Refused::Other("not saved; the sync did not stop"))
}
```

- [ ] **Step 2: Run them; they fail to compile.** `cargo test -p tdo --test write`

- [ ] **Step 3: Implement** `Store::set_lock_wait` (poll `VdirLock::try_exclusive` every 25 ms
  until the deadline) and the `Writer` changes. `WriteError::Busy` maps to `Refused::Busy` with
  `running(&config)` → `Utc::now() - started` (clamped at zero); every other error to
  `Refused::Other(failure(..))`. Update the existing tests mechanically (`Err("x".into())` →
  `Err(Refused::Other("x".into()))`).

- [ ] **Step 4: Run `cargo test --workspace`** (term.rs still compiles against `.message()`;
  fix its call site to `result.map_err(|e| e.message())` for now — Task 4 replaces it).

- [ ] **Step 5: Commit** `feat(tdo): wait briefly for the vdir lock and stop a sync to save`

---

### Task 4: The TUI asks to stop the sync

**Files:**
- Modify: `crates/tdo/src/app.rs` (prompt state, key handling, `Outcome::StopSyncAndWrite`,
  `fmt_elapsed`)
- Modify: `crates/tdo/src/term.rs` (handle `Refused::Busy` and the new outcome)
- Test: `crates/tdo/tests/app_keys.rs`
- Docs: `docs/tui.md` (the refused-write list, near line 129)

**Interfaces:**
- Consumes: `Writer::{run, stop_and_run}`, `Refused` (Task 3).
- Produces:

```rust
// tdo::app
pub enum Outcome { /* … */ StopSyncAndWrite(Request) }
impl App {
    /// A write found the vdir busy. With a duration: keep `req` and ask the stop question.
    /// Without: same as write_finished(Err("a sync is running; try again in a moment")).
    pub fn write_busy(&mut self, req: Request, running_for: Option<Duration>);
}
pub fn fmt_elapsed(d: Duration) -> String;   // "45s", "2m 14s", "1h 3m"
```

- [ ] **Step 1: Write the failing tests** (`crates/tdo/tests/app_keys.rs`, using its existing
  app fixture)

```rust
#[test]
fn elapsed_formats() {
    // fmt_elapsed(45 s) == "45s"; (134 s) == "2m 14s"; (3780 s) == "1h 3m"; (0) == "0s"
}

#[test]
fn busy_write_asks_to_stop_the_sync() {
    // app.write_busy(req, Some(134 s)); message ==
    //   "sync running for 2m 14s — stop it and save? (y/n)"
    // key 'y' → Outcome::StopSyncAndWrite(req); prompt gone
}

#[test]
fn n_keeps_the_form_and_its_typing() {
    // open `i`, type into the title, Enter (Save) → Outcome::Write(req)
    // write_busy(req, Some(5 s)); key 'n' → Continue
    // form still open with the typed title; form.error == Some("not saved; the sync is still running")
}

#[test]
fn esc_declines_like_n() { /* one-key action (x) path: task unchanged, message as above */ }

#[test]
fn other_keys_are_ignored_while_asking() {
    // write_busy(…, Some(5 s)); key 'j' → Continue; cursor unchanged; prompt still shown
}

#[test]
fn busy_without_a_duration_shows_the_plain_message() {
    // write_busy(req, None) → message "a sync is running; try again in a moment"; no prompt
}

#[test]
fn n_after_the_task_changed_rebuilds_the_form() {
    // as n_keeps_the_form…, but the snapshot's task changed before 'n' (Review Focus 5):
    // form shows the saved version and error ends with "the form now shows the saved version"
}
```

- [ ] **Step 2: Run them; they fail to compile.** `cargo test -p tdo --test app_keys`

- [ ] **Step 3: Implement**

- `App` holds `stop_prompt: Option<Request>`. `key_at` checks it first (before the form and
  every mode): `y` → `StopSyncAndWrite`; `n`/`Esc` → clear it and
  `write_finished(Err("not saved; the sync is still running"))`; anything else → `Continue`.
- The prompt message is not transient (it stays until answered).
- `term.rs`: `Outcome::Write(req)` → `writer.run`; on `Err(Refused::Busy { running_for })` →
  `app.write_busy(req, running_for)` (after the reload, as for other results); otherwise
  `write_finished(result.map_err(|e| e.message()))`. `Outcome::StopSyncAndWrite(req)` →
  `writer.stop_and_run`, reload, `refresh_sync`, `write_finished(result.map_err(|e| e.message()))`.
  Both waits block the UI (2 s and up to 10 s): accepted in the design.

- [ ] **Step 4: Run `cargo test --workspace`**, clippy, fmt.

- [ ] **Step 5: Docs** (`docs/tui.md`): replace the `a sync is running` bullet with the wait,
  the question, what `y` and `n`/`Esc` do, and that a stopped sync backs off and the edit syncs
  with the next good run. `python3 tools/check_links.py`.

- [ ] **Step 6: Commit** `feat(tui): offer to stop a running sync when a save is blocked`

---

### Task 5: Verify by hand and hand over

- [ ] **Step 1:** In a scratch config (`tools/demo_vdir.py` vdir, `[sync] command` = a script that
  sleeps 120 s), open the TUI, press `x` on a task: after ~2 s the question appears with a
  growing duration; `n` leaves the task open; `x` again then `y` completes it and shows
  `done · sync stopped`; `tdo status` shows `failures: 1`, `last error: sync stopped`, `pending: 1`.
- [ ] **Step 2:** Same with an edit form (`i`, change the title, Enter, `n`): the form stays with
  the typed title.
- [ ] **Step 3:** Run `tdo sync` with that script in another pane, Ctrl-C it, then save in the TUI
  and answer `y`: the orphan is killed and the save goes through.
- [ ] **Step 4:** Update AGENTS.md "Done so far" (issue #10 fixed; next: release pipeline) and
  commit `docs: issue #10 done`. The PR body closes #10.
