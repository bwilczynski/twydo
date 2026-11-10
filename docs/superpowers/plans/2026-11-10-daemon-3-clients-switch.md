# Daemon plan 3: the clients switch to the daemon. Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The TUI, `tdo op`, `tdo sync` (with `on|off`) and `tdo conflicts resolve` become
clients of `tdo daemon` over its socket. They start the daemon on demand and never write the
vdir, take the vdir lock or read `config.toml` themselves. The `Writer`, the batch waiter, the
`spawn*` helpers and the TUI's file and preference watchers are removed.

**Architecture:** The tasks fall into three groups:

- **Tasks 1–5 harden the daemon.** They fix the review items deferred from plans 1 and 2 that
  a client will hit, and add the few wire fields the TUI still reads from files today.
- **Task 6 adds one connection path in `tdo`** (`crates/tdo/src/client/`). It resolves the
  socket and instance, starts the daemon on demand, retries for about 2 s, applies the upgrade
  rule, and wraps calls with a timeout.
- **Tasks 7–12 move each client over that path.** The CLI commands block on calls. The TUI runs
  a **link**: a background thread that connects and reconnects, and hands the event loop
  snapshots, status and outcomes over a channel, so keys and drawing never wait on the socket.

**Tech Stack:** Rust 2024; `std::thread` and `mpsc`, serde, `schemars` 1.x, `libc`,
`signal-hook` 0.3, the same crates as plans 1 and 2; `tempfile` for tests. No new crates.

**Spec:** [`docs/superpowers/specs/2026-11-09-daemon-design.md`](../specs/2026-11-09-daemon-design.md)
(§5.1, §6, §7.2, §10.1, §10.3, §10.4, §12 step 1, §13, §14). Plans 1 and 2 cover the crate
layout, the lane, sessions, replay, jobs and the lock order:
[`2026-11-09-daemon-1-core-and-writes.md`](2026-11-09-daemon-1-core-and-writes.md),
[`2026-11-10-daemon-2-sync-and-jobs.md`](2026-11-10-daemon-2-sync-and-jobs.md).

## Where this plan sits

| Plan | Delivers | Clients |
|---|---|---|
| 1 (merged, PR #78) | crate, protocol, schema, lifecycle, snapshot/status/events, watcher, task writes, queue | unchanged |
| 2 (merged, PR #79) | jobs, sync methods, scheduler, conflicts, journal, lists, `input.parse`, config reload | unchanged |
| **3 (this)** | deferred hardening; on-demand start; TUI, `tdo op`, `tdo sync`, `tdo conflicts resolve` over the socket; Writer, waiter, spawn and watchers removed | **switched** |
| 4 (next, spec §12 step 2) | `check`, `export`, `status`, `conflicts`/`show`, `op check` through the daemon; `tdo daemon install`/`uninstall`/`status`; service-manager start; no-unit warning; contrib units; full docs | read-only too |

After this plan nothing but the daemon writes the vdir (spec §12 step 1). Read-only commands
still read files directly until plan 4. This is safe under the reader lock, and they still read
`config.toml`, which is the one remaining D13 exception (spec §12 says so).

## Decisions this plan makes (builder's calls in the spec)

- **Read-only commands wait for plan 4.** `tdo check`, `tdo export-json`, `tdo status`,
  `tdo conflicts` (list), `tdo conflicts show` and `tdo op check` are unchanged here.
  `tdo op journal` and `tdo op recover` move now, because `journal.list` and `journal.recover`
  exist. `tdo conflicts hook` stays a direct process, as a child of the daemon's vdirsyncer
  (spec §3).
- **On-demand start is the detached daemon only.** Starting through `systemctl` or `launchctl`
  depends on `tdo daemon install`, which is plan 4. Here a client runs
  `tdo daemon --detach --socket <path> [--vdir <path>]`: the same executable, `setsid`, stdin
  and stdout closed, and stderr appended to `$XDG_STATE_HOME/twydo/daemon-<socket-id>.log`
  (`<socket-id>` = the first 12 hex digits of the SHA-256 of the absolute socket path; use
  the existing `sha2` dependency of `twydo-core`). The client's environment is inherited.
- **`tdo daemon stop`** (new, visible) calls `daemon.shutdown {when_idle: true}` and waits up
  to 30 s for the socket to stop answering. Messages that say "restart the daemon" name it.
- **The old timer keeps working until plan 4 replaces it.** `tdo sync --background` and
  `--when-due` connect (starting the daemon on demand), say nothing and exit 0. The installed
  `twydo-sync.timer` and launchd plist then keep the daemon alive instead of syncing
  themselves. Both flags stay hidden.
- **The compatibility matrix belongs to the daemon.** `tdo op --matrix` is removed. Tests that
  need a matrix start `tdo daemon --matrix <file>` (or an in-process daemon) first.
  `tdo op check` keeps `--matrix`, since it is a local read until plan 4.
- **Call timeout.** `Caller::call_timeout(method, params, limit)` returns the new
  `ClientError::TimedOut`. CLI calls use 30 s. TUI calls run only on the link thread and use
  5 s. On a timeout the link drops the connection and reconnects. Jobs do not count against
  the limit: `tdo sync` waits for `job.finished`, checking `job.get` every 5 s while it waits.
- **The TUI link** (`crates/tdo/src/term/link.rs`) is one thread that owns the `Client`.
  The loop sends it `Command`s and receives `Update`s; nothing on the UI thread blocks on a
  socket. The link re-fetches the snapshot after `data.changed` (unless it already holds that
  revision), after `missed`, and after its own writes. It re-fetches status and the conflict
  list after `status.changed`. After a disconnect it reconnects with backoff (250 ms doubling
  to 2 s), resuming its session. An epoch change sends `Update::Restarted`.
- **TUI writes stay synchronous from the user's point of view**, as today: the loop sends
  `Command::Write`, and the next key is not handled until the reply arrives or 5 s pass. This
  is what the `Writer` does now, and the edit card depends on it. A timeout is reported as
  `daemon did not answer; try again`, and the card keeps its text.
- **The App reads list and backup state through a `Lookup` trait** (`crates/tdo/src/app/lookup.rs`)
  instead of calling `twydo_core::retired` and `lists::{backup, operation}` directly.
  Production uses `DaemonLookup`, blocking calls through the link's `Caller` with a 5 s
  timeout. They are quick reads made only when the user opens a panel. Tests use
  `FileLookup`, which keeps today's file reads, so the existing recovery and list tests keep
  their setup. The App's types become the wire views (`ForgetPreview`, `RetiredBackupView`,
  `WireDeletePreview`, `WireSnapshot`).
- **Refusals reach the TUI only from `status.refusals`.** The TUI shows each one once per run,
  joined with ` · ` as `drain_and_reload` does today, then calls `refusals.ack` with their ids.
  The daemon already drops refused steps from the session's undo stacks.
- **Sync messages.** `sync_message(&SyncResult) -> String` in `crates/tdo/src/term/link.rs`
  builds the text:
  - problems, then warnings as `warning: <w>`, all joined with ` · `;
  - `app::SYNC_DONE` when both lists are empty and the outcome is `ok`;
  - `sync failed` when they are empty and the outcome is `failed`;
  - `sync stopped` for `stopped` and `unknown`.

  `tdo sync` prints problems as `tdo: <p>` and warnings as `tdo: warning: <w>` on stderr, as
  today.
- **Exit codes of `tdo op`** stay the same:
  - 0 when the change is applied, satisfied or queued;
  - 1 (`EXIT_BLOCKED`) for the error kinds `blocked`, `stale` and `stuck`;
  - 2 for everything else, including no daemon (`daemon not running: <reason>`).

  Output stays the same too: an applied write prints the journal id, a queued one prints
  `queued (a sync is running): <op_id>`, and with `--wait` the journal id once `op.finished`
  says `applied`.
- **Timezone joins the binding fields.** A live change to `timezone` or `work_days` is
  reported in `status.config.restart_needed` and not applied, as with `vdir` and the others.
  This fixes the half-applied drain gate (plan-2 deferred, Task 9) without moving the gate's
  "today" into each queue item.

## Global Constraints

- Contracts in the main spec §11 bind everything. "Nothing is ever planned against a newer
  snapshot than the client saw" (daemon spec §6.6): a client's `based_on` is the revision of
  the snapshot its view was built from (for the TUI, `App::write_snapshot()`'s revision).
- **No client writes the vdir, takes the vdir lock, or calls `Config::load` or
  `cli::resolve_config`.** The exceptions are `tdo daemon` itself, the plan-4 read-only
  commands listed above, and `tdo conflicts hook`. The one thing a client checks without
  reading is whether `Config::path()` exists. With neither that file nor `--vdir`, it fails at
  once with today's `CliError::NoConfig` text (spec §10.1).
- On-disk data is unchanged, apart from the optional fields plans 1 and 2 added.
- Pure modules (`ical`, `model`, `views`, `dateinput`, `repeatinput`, `edit`, `gate`) stay
  free of file, process and clock access.
- **Every test that runs `tdo` or starts a daemon sets its own socket** (`TWYDO_SOCKET`, a short
  temp dir from `short_socket_dir()`), so no test can reach or start a daemon at the user's
  default socket (spec §10.4). **No test runs the real `vdirsyncer`.**
- Mutating methods carry a non-empty `request_id` from `Client::next_request_id()`. A resent
  call reuses the same id. This includes the TUI's retry after a reconnect, which resends the
  write it had in flight (the replay rule, spec §6.6).
- Enums that clients receive are open (`#[schemars(transform = super::open_enum)]` plus an
  `#[serde(other)] #[schemars(skip)] Unknown` variant), including every enum this plan adds.
  Clients treat `Unknown` as spec §6.2 says.
- Message wording follows today's (`tdo op`, `tdo sync`, `tdo conflicts resolve`, the TUI). A
  message that used to say "restart Twydo" now says
  `restart the daemon (tdo daemon stop) and reopen tdo`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass at every commit.
- Every task that changes a protocol type regenerates `schema/tdo.openrpc.json`
  (`TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`), and the breaking-change test
  against `schema/released.openrpc.json` passes.
- Use the default cargo target directory. A second target dir must sit on the home disk
  (`~/.cache/…`), never under `/tmp`.
- Conventional commits, with no agent attribution lines. No personal data: fixtures are
  synthetic.

## Review Focus

1. **The TUI must not freeze when the daemon dies mid-session.** Keys keep working, the status
   line says `daemon not running: …; retrying`, and the link reconnects when the daemon comes
   back. Test: `tui_keeps_running_while_the_daemon_is_down` (Task 11).
2. **A write in flight when the connection drops must be applied once.** The link resends the
   same `request_id` after reconnecting and reports the original outcome. Test:
   `a_write_resent_after_a_reconnect_applies_once` (Task 10).
3. **A test command must never touch the user's production daemon.** A client whose
   `XDG_CONFIG_HOME` differs from the daemon's gets `wrong_instance`, does not start, stop or
   restart that daemon, and exits 2. Test: `wrong_instance_never_restarts_the_daemon` (Task 6).
4. **Two clients starting a daemon at once leave one daemon**, and a stale socket file left by
   a daemon of another vdir is not removed while that daemon still answers. Test:
   `two_clients_starting_at_once_leave_one_daemon` (Task 6).
5. **The edit card keeps the user's typing** when a save is refused as `stale` through the
   daemon, as it does today with the in-process `Writer`. Test:
   `sync_while_form_open_is_stale_not_overwritten`, ported (Task 10).

---

### Task 1: Request safety in the daemon (plan-1 deferred)

A client now relies on every request getting an answer.

- Also fold in the rows of [Deferred review items](#deferred-review-items-what-this-plan-does-with-each) assigned to this task; each gets a test where the row names one.

**Files:**
- Modify: `crates/twydo-daemon/src/server/conn.rs` (request threads, outbound writes),
  `src/server/sessions.rs` (poisoning, I/O outside the table lock)
- Test: `crates/twydo-daemon/tests/debt.rs`

**Interfaces:**
- Produces: a panicking handler answers `ErrorKind::Internal` with the message
  `internal error: <panic text>`, and the connection stays usable. Session-table methods never
  panic on a poisoned mutex.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_panicking_handler_answers_internal() {
    // DaemonOptions::test_point = Some(TestPoint::PanicIn("status.get")) (new variant);
    // status.get → Err kind Internal, message starts "internal error"; daemon.info on the same
    // connection still answers.
}
#[test] fn a_poisoned_session_table_keeps_serving() {
    // TestPoint::PoisonSessions: a hook panics while holding the sessions mutex; afterwards a
    // fresh client's hello and op.submit succeed.
}
#[test] fn a_client_that_stops_reading_does_not_pin_the_daemon() {
    // raw client subscribes and never reads; publish 10_000 events (reload_for_test in a loop);
    // within 10 s the daemon closes that connection (write timeout) and another client's
    // daemon.info answers in < 1 s.
}
#[test] fn session_recovery_io_runs_outside_the_table_lock() {
    // TestPoint::SlowWithoutRecovery(2 s) delays Store::without_recovery in session open;
    // a second client's hello on another session answers in < 500 ms meanwhile.
}
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test debt`: all four
  FAIL (the panic test hangs until the harness limit, so put a 5 s `recv_timeout` in it).
- [ ] **Step 3: Implement.**
  - Wrap each request handler in `std::panic::catch_unwind(AssertUnwindSafe(..))` and map a
    panic to `RpcError::refused(ErrorKind::Internal, format!("internal error: {msg}"))`.
  - `Sessions` locks with `.lock().unwrap_or_else(PoisonError::into_inner)`.
  - Open the `Store::without_recovery` before taking the table lock, then insert.
  - Outbound writes set a 5 s `set_write_timeout`. A timed-out write closes the connection
    with `Shutdown::Both`, which also replaces the `Shutdown::Read`-only close.
- [ ] **Step 4: Run them and check they pass.** `cargo test -p twydo-daemon`: PASS.
- [ ] **Step 5: Commit.** `fix(daemon): answer panicking requests and drop clients that stop reading`

### Task 2: Replay and `op.get` correctness (plan-1 deferred)

- Also fold in the rows of [Deferred review items](#deferred-review-items-what-this-plan-does-with-each) assigned to this task; each gets a test where the row names one.

**Files:**
- Modify: `crates/twydo-daemon/src/server/replay.rs`, `src/server/writes.rs`,
  `crates/twydo-core/src/store/journal.rs` (index by request id)
- Test: `crates/twydo-daemon/tests/writes.rs`

**Interfaces:**
- Consumes: `Journal::by_request(&self, request_id: &str) -> io::Result<Option<Entry>>` (exists,
  scans the whole journal).
- Produces:
  - `Journal::by_request` is served from an index file,
    `<state>/journal/by-request/<sha256(request_id)[..16]>`, which holds the entry id and is
    written in the same commit step as the entry. A missing index falls back to the scan.
  - A replayed `create` answers `created: Some(Created { uid, rev })`, taken from the journal
    entry's uid and the file's current rev.
  - `Stale(path)` for a path outside the client's view carries `uid` from the journal plan.
  - `op.get {request_id}` of an undo that withdrew a queued item answers `Cancelled`.
  - `entry_state` answers `Pending` for an entry mid-commit, not `stuck`.
  - `journal.revert` and `op.undo` name the same, pre-edit label in their messages
    (`undone: <title before the edit>`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn replayed_create_returns_the_created_uid() { /* submit create; kill -9 path via TestDaemon restart; resend → created.uid == first.created.uid */ }
#[test] fn stale_outside_the_view_names_the_uid() { /* family op whose child file is rewritten outside → Err stale, data.uid == child uid */ }
#[test] fn op_get_by_request_of_a_cancelling_undo_is_cancelled() { /* queued edit (hold_long), undo it, op.get{request_id of the edit} == OpState::Cancelled */ }
#[test] fn entry_mid_commit_is_pending_not_stuck() { /* in_commit hook pauses; op.get{request_id} from another client == Pending */ }
#[test] fn replay_lookup_does_not_scan_the_journal() { /* 2_000 synthetic journal entries; resend of the newest request answers in < 50 ms */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test writes`
- [ ] **Step 3: Implement** the five behaviours above. The index is optional data, so a crash
  between writing the entry and writing the index only costs the fallback scan.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `fix(daemon): replay creates and cancelled undos exactly, index requests`

### Task 3: The sync and job contract (plan-2 "fix in plan 3")

- Also fold in the rows of [Deferred review items](#deferred-review-items-what-this-plan-does-with-each) assigned to this task; each gets a test where the row names one.

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs`, `crates/twydo-daemon/src/server/syncjob.rs`,
  `src/server/jobs.rs`, `src/server/schedule.rs`, `src/server/admin.rs`, `src/server/mod.rs`
  (teardown), `src/server/config.rs`
- Modify: `docs/superpowers/specs/2026-11-09-daemon-design.md` §6.5 (the contract below)
- Test: `crates/twydo-daemon/tests/sync.rs`, `tests/jobs.rs`, `tests/admin.rs`, `tests/config.rs`

**Interfaces:**
- Produces:
  - `twydo_core::sync::run::run` returns `Ok(RunReport)` with `report.error: Option<String>`
    when it drained items and then failed. The daemon reports such a run as outcome `failed`,
    with that error in `problems`, and sends one `op.finished` per drained item.
  - **The outcome-event contract**, written into spec §6.5 and tested:
    - every queued item gets exactly one `op.finished`;
    - `sync.finished` comes after the post-sync reload is published;
    - `job.finished` comes after `sync.finished`;
    - a job record reads `finished` (`job.get`) only once `job.finished` is sent;
    - `status.sync.state` stays `running` until `sync.finished` is sent.
  - A panic inside a runner job is caught. Its job ends `failed`/`internal`, `sync_running` is
    cleared and `sync.finished { result.outcome: failed }` is sent.
  - `conflicts.resolve` refuses a choice that differs from the one already recorded under
    another request, with kind `invalid` and the message
    `conflict <id> already resolved as <side>`. The same request still replays.
  - `journal.recover` calls `request_drain` after it resumes entries.
  - Schedule arithmetic uses `checked_add_signed`. An overflow means "no deadline".
  - Teardown checks `JoinHandle::is_finished` on the runner every 100 ms and stops waiting
    once the thread is gone.
  - `job.get` with neither id answers `invalid` with `give exactly one of job_id or request_id`.
  - `timezone` and `work_days` join the binding fields (`restart_needed`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_sync_that_drains_then_fails_finishes_each_drained_item() { /* fake vdirsyncer exits 1 after the drain; two queued edits → two op.finished(applied), sync.finished failed */ }
#[test] fn job_get_is_final_only_after_job_finished() { /* poll job.get in a tight loop while waiting; the first final job.get is seen after sync.finished and job.finished arrived */ }
#[test] fn status_reports_running_until_sync_finished() { /* every status.get between sync.started and sync.finished says Running */ }
#[test] fn a_panicking_sync_job_ends_failed_and_clears_running() { /* TestPoint::PanicInSync; job.finished failed/internal; status.sync.state == Idle */ }
#[test] fn a_differing_rechoice_is_refused() { /* resolve A keep local (req1) then keep remote (req2) → invalid; resend req1 → original result */ }
#[test] fn journal_recover_drains_items_held_behind_a_stuck_entry() { /* stuck entry + queued item; journal.recover after fixing → op.finished applied */ }
#[test] fn huge_periodic_does_not_panic() { /* DaemonOptions::periodic = Duration::MAX / 2; scheduler thread alive after 1 s */ }
#[test] fn timezone_change_needs_a_restart() { /* edit config timezone → status.config.restart_needed == ["timezone"]; snapshot.config.timezone unchanged */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon`
- [ ] **Step 3: Implement** the behaviours listed under Interfaces. In `run_sync`, move
  `jobs.set_state(Finished)` after `SyncDone::finish()`. Keep `running` in the stop-gate state
  and clear it in `SyncDone::finish`.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `fix(daemon): keep the outcome-event order and finish drained items of a failed sync`

### Task 4: What the TUI still reads from files goes on the wire

- Also fold in the rows of [Deferred review items](#deferred-review-items-what-this-plan-does-with-each) assigned to this task; each gets a test where the row names one.

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/methods.rs` (`SyncView`),
  `src/protocol/lists.rs`, `src/protocol/admin.rs` (`InputParseParams`), `src/protocol/wire.rs`,
  `src/server/publish.rs`, `src/server/lists.rs`, `src/server/admin.rs`, `src/server/dispatch.rs`,
  `src/schema.rs` (method table)
- Test: `crates/twydo-daemon/tests/read.rs`, `tests/lists.rs`, `tests/wire.rs`

**Interfaces:**
- Produces:
  - `SyncView.collections: BTreeMap<String, WireCollState>` (`serde(default)`).
    `WireCollState` is an open enum, `{ok, conflict, emptied_local, emptied_remote,
    error{message}}`, with `From<&CollState>`. Also `SyncView.metasync_error: Option<String>`.
  - New method **`lists.pending`** (no params) returns `Option<PendingListOp>`:

    ```rust
    pub struct PendingListOp {
        pub operation: String,
        pub action: String,             // "create" | "update" | ...
        pub collection: Option<String>,
        pub name: Option<String>,
        pub color: Option<String>,
        pub observed_name: Option<String>,
        pub observed_color: Option<String>,
    }
    ```

    It is built from `lists::operation::active`, with the observed values taken the way
    `tdo::app::lists::observed` takes them. Move `observed` to
    `twydo_core::lists::operation::observed` so both use it.
  - `pub fn ForgetPreview::of(&retired::Preview) -> ForgetPreview`,
    `RetiredBackupView::of(&retired::Backup)` and `WireDeletePreview::of(&backup::Backup)`, made
    public for `FileLookup` (Task 12).
  - `InputParseParams.anchor: Option<NaiveDate>` (`serde(default)`): the repeat description
    uses it. The TUI passes `due.or(today)`.
  - The wire sends `rev: ""` only for a task whose bytes are unknown, and `from_wire` keeps
    that. Round-trip tests cover a task with a VALARM, an RRULE with UNTIL, a zoned due date
    and a date-only due date.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn status_carries_per_list_sync_state() { /* status.json with list "inbox" Error{"…Not Found"} → status.sync.collections["inbox"] == Error{message} */ }
#[test] fn lists_pending_reports_an_unfinished_list_save() { /* seed an active operation record → lists.pending == Some(PendingListOp{action:"update", collection:Some("inbox"), observed_name:Some(..), ..}) */ }
#[test] fn lists_pending_is_none_when_idle() {}
#[test] fn input_parse_describes_a_monthly_repeat_from_the_anchor() { /* repeat "monthly", anchor 2026-11-14 → description contains "14" */ }
#[test] fn wire_round_trip_keeps_alarms_rrule_and_zoned_due() { /* to_wire→JSON→from_wire; Task fields equal for the four fixtures */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon`
- [ ] **Step 3: Implement**, then regenerate the schema.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(daemon): per-list sync state, lists.pending and an input.parse anchor`

### Task 5: Schema guard and the release step (plan-1 deferred)

**Files:**
- Modify: `crates/twydo-daemon/src/schema.rs` (breaking-change checker), `tests/schema.rs`
- Modify: `.github/workflows/` release-plz workflow (the job that opens release PRs)
- Create: `tools/refresh_released_schema.sh`

**Interfaces:**
- Produces:
  - The checker compares lifecycle methods ignoring `description` fields, matches `allOf`
    parts by content rather than by index, and accepts `integer` widened to `number` in params
    (and flags it in results).
  - The release PR job runs `tools/refresh_released_schema.sh`, which copies
    `schema/tdo.openrpc.json` over `schema/released.openrpc.json`, and commits the copy to the
    release PR branch.

- [ ] **Step 1: Write the failing tests** (`tests/schema.rs`):
  `lifecycle_description_change_is_not_a_break`, `reordered_all_of_is_not_a_break`,
  `integer_to_number_in_params_is_not_a_break`, `integer_to_number_in_results_is_a_break`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test schema`
- [ ] **Step 3: Implement** the checker changes and the script, and add the workflow step.
- [ ] **Step 4: Check.** The tests pass, and `bash tools/refresh_released_schema.sh && git diff --quiet schema/`
  passes on a clean tree (the schemas are equal now).
- [ ] **Step 5: Commit.** `ci: refresh the released protocol schema in release PRs`

### Task 6: Connecting, on-demand start and the upgrade rule

**Files:**
- Create: `crates/tdo/src/client/mod.rs`, `crates/tdo/src/client/start.rs`
- Modify: `crates/twydo-daemon/src/client.rs` (`call_timeout`, `TimedOut`),
  `crates/twydo-daemon/src/server/mod.rs` (stale-socket check),
  `crates/tdo/src/cli/daemon.rs` (`--detach`, `stop`), `crates/tdo/src/main.rs`, `crates/tdo/src/lib.rs`
- Create: `crates/tdo/tests/common/mod.rs` (shared test helpers)
- Test: `crates/tdo/tests/daemon_process.rs`

**Interfaces:**
- Consumes: `twydo_daemon::paths::{socket_path, instance}`, `Client::connect`, `Client::connect_raw`.
- Produces:

```rust
// crates/tdo/src/client/mod.rs
pub struct Target { pub socket: PathBuf, pub vdir: Option<PathBuf> }   // from --socket / --vdir
pub enum ConnectError {
    NoConfig(PathBuf),               // Display: CliError::NoConfig's text
    WrongInstance(String),           // the daemon's message
    TooOld { client: String, daemon: String },  // Display: "tdo was upgraded; restart it"
    Unavailable(String),             // Display: "daemon not running: {0}"; when a daemon this
                                     // call started has exited, {0} ends with the last line of its log
}
pub fn connect(t: &Target, client: &str, session: Option<String>) -> Result<(Client, HelloResult), ConnectError>;
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);
// crates/twydo-daemon/src/client.rs
impl Caller { pub fn call_timeout<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P, limit: Duration) -> Result<R, ClientError>; }
// ClientError::TimedOut — Display "the daemon did not answer within {secs} s"
```

  `connect` works in this order:
  1. If `Config::path()` is missing and there is no `--vdir`, return `NoConfig`.
  2. Try to connect. On a missing socket or a refused connection, start the daemon
     (`start::detached(t)`) and retry every 100 ms for up to 2 s.
  3. On `hello` → `protocol`: if the client's version is higher than `data.daemon` (semver
     compare), call `daemon.shutdown {when_idle: true}` on a raw connection, wait for the
     socket to go, start the daemon and retry. Otherwise return `TooOld`.
  4. On `wrong_instance`, return `WrongInstance` and never start or stop anything.
  5. On the same protocol with a higher client version: say `hello` succeeds, then do the
     shutdown and restart as in 3 (spec §10.3).
- `start::detached(t: &Target) -> io::Result<()>` spawns as described under Decisions.
  `start::log_path(socket: &Path) -> PathBuf`.
- `tdo daemon --detach` re-execs itself without `--detach` under `setsid`, then exits 0.
  `tdo daemon stop` connects without starting anything. With nothing running it prints
  `no daemon is running` and exits 0.
- Server: before removing a leftover socket file, the daemon also refuses when a daemon that
  answers is serving another vdir. It exits 2 with
  `<socket> is served by a daemon for <vdir>; use another --socket`.
- `tests/common/mod.rs`:
  - `short_socket_dir() -> TempDir`
  - `tdo_cmd(env: &TestEnv) -> Command`: sets `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and
    `TWYDO_SOCKET`; never inherits `TWYDO_SOCKET`
  - `TestEnv::new(config_toml: &str)`
  - `TestEnv::start_daemon(&self, extra: &[&str]) -> Child`
  - `TestEnv::stop_daemon(&self)`
  - `in_process_daemon(cfg: Config, matrix: Matrix) -> (DaemonHandle, TempDir)`

- [ ] **Step 1: Write the failing tests** (`tests/daemon_process.rs`)

```rust
#[test] fn a_client_starts_the_daemon_on_demand() { /* no daemon; `tdo op journal` exits 0; daemon.info answers; log file exists at start::log_path */ }
#[test] fn two_clients_starting_at_once_leave_one_daemon() { /* two `tdo op journal` in parallel threads both exit 0; exactly one `tdo daemon` pid in daemon.info across 3 calls; lock loser exited 10 */ }
#[test] fn wrong_instance_never_restarts_the_daemon() { /* daemon for env A; client with XDG_CONFIG_HOME of env B and A's socket → exit 2, stderr contains "not the instance"; daemon.info epoch unchanged */ }
#[test] fn no_config_fails_without_starting_anything() { /* no config, no --vdir → exit 2, NoConfig text; socket file absent */ }
#[test] fn a_newer_client_restarts_an_older_daemon_when_idle() { /* in-process daemon built with version override "0.0.1" (DaemonOptions::version_for_test); `tdo op journal` → epoch changes; exit 0 */ }
#[test] fn an_older_client_never_restarts_a_newer_daemon() { /* version_for_test "99.0.0" + protocol mismatch → exit 2 "tdo was upgraded; restart it"; epoch unchanged */ }
#[test] fn a_live_socket_of_another_vdir_is_not_taken_over() { /* daemon A on S; `tdo daemon --socket S` for vdir B exits 2; A still answers */ }
#[test] fn daemon_stop_waits_for_the_socket_to_go() {}
#[test] fn call_timeout_reports_timed_out() { /* in twydo-daemon tests: TestPoint::SlowMethod("status.get", 2 s), call_timeout 200 ms → ClientError::TimedOut */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test daemon_process`
- [ ] **Step 3: Implement** the `client` module, `start::detached`, the `--detach` and `stop`
  subcommands, `call_timeout`, and the server's stale-socket check.
  `DaemonOptions::version_for_test: Option<String>` replaces `CARGO_PKG_VERSION` in `hello`
  and `daemon.info`.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(tdo): connect to the daemon, starting it on demand`

### Task 7: `tdo op` over the socket

**Files:**
- Modify: `crates/tdo/src/cli/op.rs`, `crates/tdo/src/main.rs` (drop `op --matrix`)
- Modify: `crates/tdo/src/cli/mod.rs` (remove `permission_snapshot`'s use by `op`; keep it for `op check`)
- Test: `crates/tdo/tests/cli_op.rs` (ported to `tests/common`)

**Interfaces:**
- Consumes: `client::connect`, `CLI_TIMEOUT`, `from_wire`, `op.submit`, `op.get`,
  `journal.revert`, `journal.abandon`, `journal.list`, `journal.recover`, `status.get`,
  `refusals.ack`.
- Produces: `fn run(target: &client::Target, cmd: OpCommand) -> i32`, with `op check` keeping
  `matrix` (and its own entry point `check_command(vdir, matrix, uid) -> i32`).
  - **Order for a write:** connect; `snapshot.get`, then `from_wire` (the dates and repeat rules
    of `add` and `edit` are parsed with `snap.config`); `status.get`, printing each refusal
    without a session or whose request id starts with `op-` as `tdo: <message>` and acking
    them; then `op.submit { session, request_id, based_on: {epoch, revision}, op }`.
  - **Queued `--wait`:** print `waiting for the sync to finish (queued <op_id>)` once, wait for
    `op.finished` with that `op_id`, and fall back to `op.get {op_id}` every 5 s and after
    `missed` or a reconnect. After a reconnect the session is gone, and that is fine: `op.get`
    needs none.
  - `refused` exits 1 with its message. `cancelled` exits 1 with
    `queued write <id> was cancelled before it was applied`.
  - `tdo op undo <id>` uses `journal.revert`, `abandon` uses `journal.abandon`, `recover` uses
    `journal.recover` (print `resumed: N`, `stuck: N` and the stuck lines as today), and
    `journal` uses `journal.list` (one line per entry in today's format).
  - The test-write-mode warning is printed when `snap.config.writes.mode == Test`.

- [ ] **Step 1: Port the tests.**
  - Every `tdo op` test runs through `tests/common::tdo_cmd`.
  - Tests that passed `--matrix` now start `TestEnv::start_daemon(&["--matrix", path])` first.
  - Tests that faked "a sync holds the vdir" with a `VdirLock` now hold the daemon's lane:
    start a fake vdirsyncer that sleeps, then run `tdo sync` in the background.
  - Add:

```rust
#[test] fn op_wait_survives_a_daemon_restart() { /* queue an edit behind a sleeping fake sync; `tdo op edit --wait` in background; kill -9 the daemon; restart it; the waiting command exits 0 printing the journal id */ }
#[test] fn op_never_opens_the_config_file() { /* start the daemon, chmod 000 config.toml, `tdo op add` exits 0 */ }
#[test] fn op_prints_refusals_of_ended_cli_sessions_once() {}
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test cli_op`
- [ ] **Step 3: Implement.** Delete `wait_for`, `sync_after_write`, `Failure: From<WriteError>`
  and the `Store` use from `op.rs`. Map an RPC error's `data.kind` to exit 1 or 2 as described
  under Decisions.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(tdo): tdo op talks to the daemon`

### Task 8: `tdo sync`, `sync on|off` and `conflicts resolve` over the socket; the waiter goes

**Files:**
- Modify: `crates/tdo/src/cli/sync.rs`, `crates/tdo/src/cli/conflicts.rs` (`resolve_cmd` only),
  `crates/tdo/src/main.rs`
- Delete from `crates/tdo/src/cli/sync.rs`: `background_command`, `detach`, `spawn_background`,
  `spawn`, `start`, `set_switch`, `print_refusals`
- Delete from core if no caller is left: `twydo_core::sync::batch::wait_and_sync` and
  `SystemClock` (if only the waiter used them), and `twydo_core::sync::ended` (read by the
  TUI's `EndedWatch`, which goes in Task 11). Check with `cargo build` plus
  `grep -rn "ended::\|wait_and_sync" crates/` before deleting.
- Test: `crates/tdo/tests/sync_e2e.rs` (ported), `crates/tdo/tests/cli.rs`

**Interfaces:**
- Consumes: `sync.run`, `job.get`, `sync.set_auto`, `conflicts.resolve`, `client::connect`.
- Produces:
  - `tdo sync [--force-delete L…]`: connect, `events.subscribe`, then
    `sync.run {request_id, force_delete}`, then wait for `job.finished` for that job (falling
    back to `job.get` every 5 s). Print problems and warnings as described under Decisions.
    Exit 0 on `ok`, 1 on `failed` or `stopped` (today's codes), 2 on a refused call.
    `NOT_CONFIGURED` comes from the daemon as `invalid`, with the same text.
  - `tdo sync --background` and `--when-due`: connect and exit 0 silently. A connect error
    exits 2.
  - `tdo sync on|off`: `sync.set_auto`, printing `result.message`.
  - `tdo conflicts resolve <id> --keep <side>`: `conflicts.resolve`, printing
    `Resolution::message()` text from the result. The daemon schedules the follow-up sync.

- [ ] **Step 1: Port the tests.**
  - `sync_e2e` sets `TWYDO_SOCKET` per `Pair` and starts each side's daemon on demand.
  - Tests that assumed `tdo sync` ran vdirsyncer in-process still pass: the fake script is in
    the daemon's config.
  - Tests of the waiter, `ended` and `spawn_background` are deleted; their behaviour is
    covered by the daemon's `tests/schedule.rs`.
  - Add:

```rust
#[test] fn sync_waits_behind_a_running_sync_instead_of_failing() { /* fake sync sleeps 2 s; two `tdo sync` in parallel both exit 0 */ }
#[test] fn background_flag_only_keeps_the_daemon_alive() { /* `tdo sync --background` exits 0, prints nothing, daemon.info answers, no extra sync.started within 1 s beyond the scheduler's */ }
#[test] fn sync_switch_goes_through_the_daemon() { /* `tdo sync off` → status.sync.auto == false */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test sync_e2e --test cli`
- [ ] **Step 3: Implement**, then delete the waiter path and its tests.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(tdo): tdo sync and conflict resolution through the daemon; remove the batch waiter`

### Task 9: The TUI link

**Files:**
- Create: `crates/tdo/src/term/link.rs`
- Modify: `crates/tdo/src/term.rs` (module only), `crates/tdo/src/app.rs` (`SyncSummary` source)
- Create: `crates/tdo/src/app/sync_state.rs`
- Test: `crates/tdo/tests/link.rs` (new, in-process daemon)

**Interfaces:**
- Consumes: `client::connect`, `Caller::call_timeout`, `from_wire`, `StatusView`, `ConflictList`.
- Produces:

```rust
pub struct Link { /* tx: Sender<Command>, rx: Receiver<Update>, caller: Arc<Mutex<Option<Caller>>> */ }
impl Link {
    pub fn start(target: client::Target) -> Link;           // connects on its thread
    pub fn send(&self, c: Command);
    pub fn try_updates(&self) -> std::sync::mpsc::TryIter<'_, Update>;
    pub fn caller(&self) -> Option<Caller>;                 // for DaemonLookup (Task 12)
    pub fn based_on(&self) -> Option<BasedOn>;              // epoch + revision of the last snapshot sent
}
pub enum Command { Write(WriteRequest), Sync { force_delete: Vec<String> }, SetAuto(bool),
                   Resolve { id: String, keep: WireKeep }, Forget { name: String, collection: String },
                   List(ListAction), Ack(Vec<String>), Quit }
pub enum Update {
    Connected { epoch: String, restarted: bool },
    Down(String),                                  // "daemon not running: …; retrying"
    Snapshot { revision: u64, snapshot: Snapshot, cause: Cause, uids: Vec<String> },
    Status(ClientSync),
    Written(Result<String, String>),               // reply to Command::Write
    SyncDone(String),                              // sync_message(...)
    Forgot(String),
    ListDone(ListCompletion),
}
pub fn sync_message(r: &SyncResult) -> String;
```

```rust
// crates/tdo/src/app/sync_state.rs — what the status line and the conflict overlay read
pub struct ClientSync {
    pub sync: Option<SyncView>, pub unsynced: usize, pub queued: usize,
    pub conflicts: Vec<ConflictView>, pub refusals: Vec<RefusalView>, pub config: ConfigStatus,
    pub drain_error: Option<String>,
}
```

  `App::sync` becomes `Option<ClientSync>` (`None` without `[sync]`, that is when
  `status.sync` is `None`). `ui/status.rs` and `ui/conflicts.rs` read it:
  - `s.off` becomes `!sync.auto`;
  - `s.status.collections` becomes `sync.collections`;
  - `s.pending + s.queued` becomes `unsynced + queued`;
  - conflicts become `ConflictView` (title, uid, held, state, choice).

  The `SyncActivity` input changes from the `running.json` probe to
  `sync.state == Running`. `App::conflicts()` returns `&[ConflictView]`.
- `WriteRequest` is `tdo::write::Request` (Task 10) plus `based_on: BasedOn`.

- [ ] **Step 1: Write the failing tests** (`tests/link.rs`, in-process daemon)

```rust
#[test] fn link_delivers_the_first_snapshot_and_status() {}
#[test] fn an_outside_change_arrives_as_a_snapshot_with_cause_outside() { /* handle.reload_for_test_with(Outside, [uid]) → Update::Snapshot{cause: Outside, uids} */ }
#[test] fn link_reconnects_and_reports_a_restart() { /* stop the daemon → Update::Down within 2 s; start a new one on the socket → Connected{restarted: true} then Snapshot */ }
#[test] fn missed_events_refetch_snapshot_and_status() { /* event_buffer 1, burst of reloads → a Snapshot with the latest revision */ }
#[test] fn sync_message_matches_tdo_sync_wording() { /* problems+warnings joined " · ", SYNC_DONE, "sync failed" */ }
```

  Port `ui_render.rs` and `app_keys.rs` tests that set `app.sync` to build `ClientSync`.
  Their expected text does not change.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test link`
- [ ] **Step 3: Implement the link thread.** It runs one loop: try to connect; then
  `select`-like polling (100 ms) over the client's notification receiver and the command
  receiver; on a dropped connection, send `Down` and back off. A write that was in flight
  when the connection dropped is resent with the same `request_id` once the link reconnects.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(tdo): a link thread that feeds the TUI from the daemon`

### Task 10: The Writer becomes a thin adapter over the link

**Files:**
- Modify: `crates/tdo/src/write.rs` (rewrite)
- Test: `crates/tdo/tests/write.rs`, `crates/tdo/tests/item_order.rs` (ported to an in-process daemon)

**Interfaces:**
- Consumes: `Link` and `Command::Write`, from Task 9.
- Produces:
  - `pub enum Request { Apply(Op), Recover { backup: String, to: String }, Undo, Redo }`
    (unchanged).
  - `pub fn execute(caller: &Caller, session: &str, based_on: &BasedOn, req: &Request, request_id: &str) -> Result<String, String>`
    runs on the link thread:
    - `Apply` → `op.submit`, `Undo` → `op.undo`, `Redo` → `op.redo`;
    - `Recover` → the `lists.recover` job: wait for its `job.finished` and return
      `Recovered.message`.

    `Ok` carries the daemon's `WriteResult.message`, which already has the queued note.
    `Err` is the error message, except that a `stale` refusal carrying `data.uid` becomes
    `"<title>" changed on disk; reload and try again`, with the title looked up in the
    client's snapshot (as `failure()` does today).
  - `Writer`, `SyncTrigger`, `LOCK_WAIT`, `Refused`, `Titled` and `step` are removed.
    `op_title` stays re-exported.

- [ ] **Step 1: Port the tests.**
  - `tests/write.rs` builds `Env` and an in-process daemon (`tests/common::in_process_daemon`
    with `Env::config()` and the fixture matrix), then drives
    `execute(&caller, &session, &based_on, &req, &id)`.
  - Each assertion on messages and file bytes is kept.
  - Tests of trigger firing (`first_save_starts_the_waiter`,
    `fifth_save_starts_the_waiter_until_saves_pause`, `refused_save_triggers_nothing`,
    `no_sync_section_triggers_nothing`, `switched_off_triggers_nothing`, the four
    `*_resolve_*` ones) are deleted. The daemon's `tests/schedule.rs` covers them; note this in
    the commit message.
  - `recover_while_syncing_says_try_again` becomes `recover_during_a_sync_runs_after_it`.
  - Add:

```rust
#[test] fn a_write_resent_after_a_reconnect_applies_once() { /* in_commit hook drops the connection after commit; the link resends the same request_id after reconnecting; one journal entry, Written(Ok("saved")) */ }
#[test] fn undo_after_a_daemon_restart_says_nothing_to_undo() { /* new epoch → op.undo → "nothing to undo" message as the daemon words it */ }
```

- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test write --test item_order`
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `refactor(tdo): the TUI writes through the daemon; remove Writer`

### Task 11: The TUI loop on the link

**Files:**
- Modify: `crates/tdo/src/term.rs` (event loop), `crates/tdo/src/main.rs` (TUI start),
  `crates/tdo/src/app.rs` (`write_snapshot` revision, messages)
- Delete: `crates/tdo/src/term/preferences.rs`; from `term.rs`: `watch`, `is_change`, `reload`,
  `reload_config`, `EndedWatch`, `poll_ended`, `poll_sync`, `drain_and_reload`, `refresh_sync`,
  `start_sync`, `run_sync_child`, `sync_child_result`, `start_forget`
- Delete from `crates/tdo/src/cli/mod.rs`: `load_snapshot`, `recover_at_startup`, `load_vdir`
  (if only the TUI used it; `check` and `export` still need a direct load, so move what they
  use into `cli/check.rs` and `cli/export.rs` as `local_snapshot(vdir)`, for plan 4 to remove).
- Test: `crates/tdo/tests/term.rs`, `crates/tdo/tests/app_keys.rs` (port where they used the
  removed functions)

**Interfaces:**
- Consumes: `Link`, `Update`, `Command`, `write::Request`.
- Produces:
  - `term::run(app: App, link: Link, tz: Tz) -> io::Result<()>`.
  - `main` builds the `Link` from `Target`. If the first `Update` within 2 s is not
    `Snapshot`, it starts the TUI on an empty snapshot (`Snapshot::empty(Config::with_vdir(<vdir flag or "">))`,
    a new pure constructor) with the `Down` message showing.
  - `App` gains `snapshot_revision: u64`, set on each `Update::Snapshot`. Each snapshot built
    for the edit card keeps its own revision, so `write_snapshot()` comes with the revision it
    was built from: `App::write_based_on(&self) -> u64`.
  - How the loop handles each `Update`:

    | Update | The loop does |
    |---|---|
    | `Snapshot` | `app.reload(snapshot)` |
    | `Status` | `app.set_sync(...)`; show unseen `refusals` and send `Command::Ack`; show `config.error` / `restart_needed` once per change |
    | `Down(m)` | show `m` until `Connected` |
    | `Connected { restarted: true }` | show `daemon restarted` once |
    | `Written(r)` | `app.write_finished(r)` (after `write_refused` on `Err`) |
    | `SyncDone(m)` | `app.sync_finished(m)` unless it is `SYNC_DONE` |
    | `Forgot(m)` | `app.message = Some(m)` |
    | `ListDone(c)` | `app.list_finished(c)` |

  - How each `Outcome` maps to a `Command`:

    | Outcome | Command |
    |---|---|
    | `Write` | `Write` |
    | `Sync` | `Sync` |
    | `SyncSwitch` | `SetAuto` |
    | `Resolve` | `Resolve` |
    | `ForgetList` | `Forget` |
    | `ManageList` | `List` |

    The "sync already running" refusals are removed, because `sync.run` waits its turn
    (spec §8).
  - Configuration warnings come from the first snapshot's `config_warnings`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn tui_keeps_running_while_the_daemon_is_down() { /* term loop driven headless (TestBackend, see tests/term.rs pattern): stop the daemon; feed 'j' keys; each handled < 50 ms; status shows "daemon not running"; restart → message cleared and rows back */ }
#[test] fn a_list_preference_change_reaches_the_tui_without_a_restart() { /* edit [lists] in config.toml → Update::Snapshot cause Config → sidebar order changes */ }
#[test] fn a_binding_change_tells_the_user_to_restart_the_daemon() { /* change vdir in config.toml → message contains "restart the daemon (tdo daemon stop)" */ }
#[test] fn refusals_are_shown_once_and_acknowledged() {}
#[test] fn tui_never_opens_the_config_file() { /* in-process daemon, chmod 000 config.toml, headless TUI loads rows and saves an edit */ }
```

  Port the `term.rs` unit tests that covered sync completion messages
  (`a_failed_sync_keeps_its_message_after_the_reload`,
  `sync_completion_refreshes_an_emptied_list_without_watcher_events`) to the `Update` path.
  Delete the ones about `EndedWatch`, `run_sync_child` and the reader lock.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo`
- [ ] **Step 3: Implement.** To make headless driving possible, split `event_loop` into
  `step(app, link, event: Option<Event>, now) -> Flow`.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS. Then run
  `tdo` by hand against a test vdir (`TWYDO_SOCKET` and `XDG_CONFIG_HOME` set, config with a
  fake vdirsyncer): open it, edit, undo, `:sync`, `:sync off`, quit.
- [ ] **Step 5: Commit.** `feat(tdo): the TUI runs on the daemon; remove the file and preference watchers`

### Task 12: List and recovery panels through the daemon

**Files:**
- Create: `crates/tdo/src/app/lookup.rs`
- Modify: `crates/tdo/src/app/recovery.rs`, `crates/tdo/src/app/lists.rs`, `crates/tdo/src/app.rs`
  (`App::lookup: Box<dyn Lookup>`, set in `App::new` to `FileLookup` of the snapshot config;
  `main` replaces it with `DaemonLookup`), `crates/tdo/src/term/lists.rs` (rewritten: `ListAction`
  → job method), `crates/tdo/src/ui/recovery.rs`, `crates/tdo/src/ui/listcard.rs` (types)
- Test: `crates/tdo/tests/recovery.rs`, `crates/tdo/tests/lists.rs` (ported), plus one
  daemon-backed test each

**Interfaces:**
- Consumes: `lists.preview_forget`, `lists.backups`, `lists.backup_preview`, `lists.pending`,
  the `lists.*` jobs, and `ForgetPreview::of`, `RetiredBackupView::of`, `WireDeletePreview::of`
  (Task 4).
- Produces:

```rust
pub trait Lookup {
    fn forget_preview(&self, name: &str) -> Result<ForgetPreview, String>;
    fn retired(&self) -> Result<Vec<RetiredBackupView>, String>;
    fn retired_preview(&self, id: &str) -> Result<Snapshot, String>;   // from WireSnapshot
    fn deleted_backups(&self) -> Result<Vec<WireDeletePreview>, String>;
    fn pending_list_op(&self) -> Result<Option<PendingListOp>, String>;
}
pub struct FileLookup(pub Config);       // today's file reads, for tests
pub struct DaemonLookup(pub Caller);     // call_timeout 5 s
```

  - `Outcome::ForgetList(ForgetPreview)`, `Confirmation::Forget(ForgetPreview)`, and
    `RecoveryPanel { backups: Vec<RetiredBackupView>, preview: Option<Snapshot>, .. }`.
  - `retired::collection_id` becomes a lookup in `app.snapshot.collections` (id first, then
    display name), with today's error text.
  - `perform_list_action` becomes `list_job(caller, action) -> ListCompletion`. It runs on the
    link thread, starts the matching `lists.*` job, waits for its `job.finished`, maps
    `JobResult` to `ListResult`, and fills `pending` and `server_values` from `lists.pending`.

- [ ] **Step 1: Port the tests.** `recovery.rs` and `lists.rs` keep `FileLookup`. Add
  `daemon_lookup_matches_file_lookup` (an in-process daemon over the same fixture: each
  `Lookup` method returns equal values from both) and
  `a_list_save_through_the_daemon_reports_its_pending_operation`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test recovery --test lists`
- [ ] **Step 3: Implement.** Check that `crates/tdo/src` has no file access left outside `cli/`:
  `grep -rn "twydo_core::retired::\|lists::backup::list\|operation::active\|Config::load\|existing_vdir_state" crates/tdo/src | grep -v "src/cli/"`
  should print nothing. The exception is `FileLookup` in `app/lookup.rs`; mark it
  `#[doc(hidden)]` with a comment that only tests use it.
- [ ] **Step 4: Run them and check they pass.** `cargo test --workspace`: PASS.
- [ ] **Step 5: Commit.** `feat(tdo): list and recovery panels read through the daemon`

### Task 13: One end-to-end TUI test, and a check that tests use their own socket

**Files:**
- Test: `crates/tdo/tests/term.rs` (end-to-end), `crates/tdo/tests/cli.rs`
- Modify: every `crates/tdo/tests/*.rs` that spawns `CARGO_BIN_EXE_tdo` without `tests/common::tdo_cmd`

**Interfaces:**
- Consumes: `tests/common`.

- [ ] **Step 1: Write the tests.**
  - `tui_end_to_end_against_a_real_daemon`: spawn `tdo` in a pty (the existing pattern in
    `tests/term.rs`) with `TWYDO_SOCKET` set; it starts the daemon on demand; type `x` on the
    first row and `q`; the task file shows `STATUS:COMPLETED`; `tdo daemon stop` exits 0.
  - `no_test_spawns_tdo_without_its_own_socket`: scan `crates/tdo/tests/*.rs` for
    `Command::new(env!("CARGO_BIN_EXE_tdo"))` outside `common/mod.rs` and fail, naming the files.
- [ ] **Step 2: Run them.** The scan fails until every call site uses `tdo_cmd`.
- [ ] **Step 3: Route** every remaining call site through `tdo_cmd`.
- [ ] **Step 4: Run** `cargo test --workspace` and check that it passes with no daemon left
  running afterwards: `pgrep -f "tdo daemon"` names no process started by the tests.
  `TestEnv` must stop its daemon on `Drop`.
- [ ] **Step 5: Commit.** `test(tdo): drive the TUI against a real daemon; every test uses its own socket`

### Task 14: Documentation and progress

**Files:**
- Modify:
  - `docs/architecture.md`: the daemon, the client link, the write path
  - `docs/sync.md`: the daemon syncs; `tdo sync` asks it to; running `vdirsyncer sync` by hand
    while the daemon runs is unsupported (spec §7.2)
  - `docs/tui.md`: the daemon status messages, `daemon restarted`, and `:sync` waiting its turn
  - `docs/config.md`: only the daemon reads the file; `timezone` and `work_days` need a
    daemon restart
  - `docs/install.md`: `tdo daemon stop` and on-demand start; the service unit comes in plan 4
- Create: `docs/protocol.md`, a short introduction to the socket, `hello`, the schema files
  and the open enums, for people writing their own clients
- Modify: `docs/README.md` (link `protocol.md`), and `AGENTS.md` (progress note: daemon plan 3
  done, plan 4 next; the test setup needs `TWYDO_SOCKET` next to `XDG_CONFIG_HOME`)

- [ ] **Step 1: Write the pages.** Describe the behaviour as it now is, in the existing pages'
  style.
- [ ] **Step 2: Check.** `python3 tools/check_links.py` passes.
- [ ] **Step 3: Commit.** `docs: the clients talk to the daemon`

---

## Deferred review items: what this plan does with each

Items marked "plan 3" in the deferred-items memories that no task above handles, each with its
decision.

| Item | Decision |
|---|---|
| Plan-2 T1: second-SIGINT test bound (2.32 s measured vs 2 s) | Task 1: tighten the bound to < 500 ms in the same commit |
| T1: `Queue::unreadable` duplicates `list_impl`'s filter | Later: cosmetic; no client effect |
| T1: `status_panic_once` prints under `--nocapture` | Dropped: test noise only |
| T1/T4/T9: lock-order docs (`before_queued_reply`, stop gate, `config_seen`, `Publisher.cfg`, `sync_now`) | Task 3: update the lock-order comment in `writes.rs` once, for all of them |
| T2: `LoadReport` cloned twice | Later: measure first; task lists are small |
| T2: stale "vdir-lock wait" comment, double `cur` | Task 3: fix while in `publish.rs` |
| T3: `JobStore` prune and interrupt fail startup on one bad record | Task 3: log the record and continue |
| T3: `JobStore::by_request` O(records) | Later: retention bounds it; revisit with metrics |
| T3: `sync.run` racing teardown leaves a `queued` record | Covered: plan 2's `unavailable` refusal while closing, and interrupted at the next start |
| T4: misplaced doc comment in `sync/report.rs` | Task 3: fix in passing |
| T4: no test for `set_auto` without `[sync]` | Task 8: `tdo sync on` without `[sync]` prints `NOT_CONFIGURED`, exit 2 |
| T4/T7: no RED-run evidence | Dropped: process note, not code |
| T4: `SyncSetAutoResult.auto` doc wording | Task 4: fix the doc |
| T5: backoff test cannot tell scheduler-idle from runner-skip | Later: test-quality only |
| T6: `when_idle` parsed twice; no race test for `when_idle` against job creation | Later: no client depends on it before plan 4's install |
| T6: run-lock shutdown test ~10 s; duplicated `write_script`/`dead_pid` | Later: test hygiene |
| T6: a job accepted just before `close()` is recorded interrupted | Accepted: the client sees `interrupted` from `job.get` and retries |
| T7: crash between restore and `mark_resolved_by` | Task 3: look the journal up by request in the replay path |
| T7: `admin.rs` re-read swallows I/O errors; `by_request` loads both `.ics`; SUMMARY extraction duplicated | Task 3: propagate the error; others later |
| T8: `journal.revert` label differs from `op.undo` | Task 7: `tdo op undo` prints the daemon's message; align both on the pre-edit label in Task 2 |
| T8: no resend test for a queued revert | Task 2: add `resent_queued_revert_replays` |
| T8: `journal_list` uses `existing_vdir_state`; `run_revert` outside the lane; recover I/O mapped to internal | Later: correct today, just indirect |
| T9: `InputField` opened needlessly; doc backwards | Task 4: fix in passing |
| T9: `set_config` outside `reload_lock` | Task 3: take `reload_lock` |
| T9: no test for default today or live `work_days` in `input.parse` | Task 4: `input_parse_defaults_to_today_in_the_configured_zone` |
| Flake: refresher blocked on an outside vdir lock at shutdown | Task 3: add `shutdown_with_a_refresher_waiting_on_an_outside_lock` |
| T10: stop-jobs wording; late `set_state` overwriting a newer daemon's `interrupted` | Task 3: fix the wording; guard `set_state` by epoch |
| T10: stale fence defers unrelated edits | Later: list jobs are rare; needs its own design |
| T10/T11: long helper sleeps in tests; double reload on forget; module doc length | Later or dropped: hygiene |
| T11: forget tests miss `sync.finished` before `job.finished`, the no-`[sync]` path and the problems suffix | Task 3: the contract test covers the order; add the no-`[sync]` case |
| T11: session mutex held during a recover job | Accepted: bounded, and only the same session waits |
| T12: §6.5 mentions `job.progress`; AGENTS.md long lines | Task 14 |
| Codex: service-unit stop timeout (`TimeoutStopSec`) | Plan 4 (the unit is written there); recorded in that plan's notes |
| Codex: watch retry test assumes non-root; downgrade drops `Conflict.earlier` | Later |
