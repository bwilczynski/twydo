# Write queue, plan 1: sync-ended event, batched sync, on/off switch — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fixed after-edit sync with batched syncs (count or age), add one persistent automatic-sync switch, and give every sync path one "sync ended" event the TUI reloads on.

**Architecture:** All rules live in `twydo-core::sync` (new modules `ended`, `switch`, `batch`, `waiter`); `tdo` only calls them and spawns detached processes. The batch waiter is `tdo sync --background --when-due`, a detached process guarded by `<vdir-state>/sync/waiter.lock`. The write queue itself is plan 2; rebasing is plan 3.

**Tech Stack:** Rust workspace (`twydo-core`, `tdo`), clap, serde/toml, chrono, ratatui; tests with `cargo test`, temp dirs and fake `sh` vdirsyncer scripts (`crates/twydo-core/tests/sync_run.rs` `Fx`).

**Spec:** `docs/superpowers/specs/2026-11-06-write-queue-design.md` (§7, §8, D2–D6, D11, D14). Read §7.2's waiter lifecycle before Tasks 5–6.

## Global Constraints

- Defaults: `batch_size = 5`, `max_wait_secs = 60`; both integers ≥ 1 (spec §7.3).
- A sync is due at `batch_size` unsynced edits, or `max_wait_secs` after the **oldest** unsynced edit; no push-back per edit (D2, D4).
- Unsynced edit = journal entry `Applied`, `synced == None`, `created >= status.last_attempt` (D14); no `last_attempt` → all pending count.
- Switch file: `<vdir-state>/sync/off` (present = off). Honoured by every `--background` run (timer, startup, waiter, size trigger); explicit `tdo sync` / `:sync` always run (D6).
- Event file: `<vdir-state>/sync/ended`, written atomically by every runner invocation that is not skipped, including ones that fail or return `Err` (§8).
- `[sync] after_edit` is no longer used: accepted, ignored, with the warning text `[sync] after_edit is no longer used: edits sync in batches (batch_size, max_wait_secs); use \`tdo sync off\` to keep syncs manual` (D5).
- Quitting the TUI starts no sync (D11); the waiter covers it.
- No personal data in tests or docs (AGENTS.md). Behaviour changes update `docs/` pages in the same task; `python3 tools/check_links.py` passes.
- Commit messages: conventional commits, no agent attribution (user's global instructions).

## Review Focus

1. **A config with `after_edit = false`** (the user's phase-0 test setup) must still load, warn once, and the user must learn that `tdo sync off` is the replacement — test in Task 3.
2. **Laptop asleep past the deadline:** on wake the deadline is in the past and the sync runs at once, not after another `max_wait` — test in Task 4 (`decide`).
3. **Old pending entries left by a sync** (a conflict, an uncounted list) must not make every save sync at once — test in Task 4.
4. **Offline with sync on:** the waiter must honour backoff (`status.next_attempt`) instead of retrying every `max_wait` — test in Task 4 and Task 5.
5. **Sync switched off while a waiter sleeps:** it wakes, sees off, exits without syncing — test in Task 5.

---

### Task 1: The "sync ended" event

**Files:**
- Create: `crates/twydo-core/src/sync/ended.rs`
- Modify: `crates/twydo-core/src/sync/mod.rs` (add `pub mod ended;`), `crates/twydo-core/src/sync/run.rs` (`run`)
- Test: `crates/twydo-core/tests/sync_run.rs`

**Interfaces:**
- Produces:
  - `pub struct Ended { pub run_id: String, pub at: DateTime<Utc> }` (serde, JSON).
  - `pub fn write(state: &Path, at: DateTime<Utc>) -> io::Result<()>` — atomic (`fsio::write_atomic`), fresh `run_id` each call (pid + nanos is enough).
  - `pub fn read(cfg: &Config) -> io::Result<Option<Ended>>` — `None` when no state dir or no file; creates nothing (use `existing_vdir_state`).

- [ ] **Step 1: Write the failing tests** in `sync_run.rs`:
  - `clean_run_writes_ended`: `run(&fx.cfg, &manual(), now())` → `ended::read(&fx.cfg).unwrap().unwrap().at == now()`.
  - `failed_run_writes_ended`: fake `exit 1` → `read` is `Some`.
  - `skipped_runs_write_no_ended`: `NotConfigured`; `Backoff` (set `status.next_attempt` in the future, background run); `Busy` (hold `sync/run.lock` via a first background run whose fake sleeps on a fifo, as `busy_background_run_requests_rerun` does) → `read` stays `None`.
  - `each_run_has_a_new_run_id`: two manual runs → different `run_id`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_run ended` — expect FAIL (module missing).
- [ ] **Step 3: Implement** `ended.rs`, and in `run()` write the event once after the pass loop **and** on the `Err` path of any pass (a small guard struct created after the backoff check whose `Drop` calls `ended::write`; ignore its error in `Drop`).
- [ ] **Step 4: Run** `cargo test -p twydo-core --test sync_run` — expect PASS (all, including existing).
- [ ] **Step 5: Commit** `feat(sync): record a sync-ended event for every run`

### Task 2: The TUI reloads on the event

**Files:**
- Modify: `crates/tdo/src/term.rs` (event loop, new `EndedWatch`)
- Test: `crates/tdo/src/term.rs` `mod tests`

**Interfaces:**
- Consumes: `twydo_core::sync::ended::read`.
- Produces: `struct EndedWatch { last: Option<String> }` with `fn new(cfg: &Config) -> EndedWatch` (records the current `run_id`) and `fn changed(&mut self, cfg: &Config) -> bool` (true once per new `run_id`; read errors → false).

- [ ] **Step 1: Write the failing test** `a_background_sync_end_reloads_without_watcher_events`: build `inbox_app`, `EndedWatch::new`, remove the task file, `ended::write(state, now)`, then call the loop's per-iteration helper `poll_ended(&mut watch, &mut app, &root)` → `app.visible_rows().is_empty()`; a second call without a new event does not reload (assert via a counter or by re-adding the file and checking rows unchanged).
- [ ] **Step 2: Run** `cargo test -p tdo --lib term::tests` — expect FAIL.
- [ ] **Step 3: Implement** `EndedWatch` and `fn poll_ended(watch: &mut EndedWatch, app: &mut App, root: &Path)` (on change: `reload` + `refresh_sync`); call it every loop iteration next to the watcher check. Keep `finish_sync` for the manual `:sync` message.
- [ ] **Step 4: Run** `cargo test -p tdo` — expect PASS.
- [ ] **Step 5: Commit** `feat(tui): reload when any sync ends`

### Task 3: Config — `batch_size`, `max_wait_secs`, retire `after_edit`

**Files:**
- Modify: `crates/twydo-core/src/config.rs` (`SyncConfig`, `Config::load`, new `Config::warnings`)
- Modify: `docs/config.md` (the `[sync]` table)
- Test: `crates/twydo-core/tests/config.rs`

**Interfaces:**
- Produces:
  - `SyncConfig { …, pub batch_size: u32, pub max_wait_secs: u64, pub after_edit: Option<bool> /* ignored */ }`; defaults 5 and 60; `after_edit` default `None`.
  - `Config::load` returns `ConfigError::Parse`-style error (new variant `ConfigError::Invalid { path, message }` if none fits) when `batch_size == 0` or `max_wait_secs == 0`, message naming the key.
  - `pub fn warnings(&self) -> Vec<String>` — the exact `after_edit` text from Global Constraints when `after_edit.is_some()`.

- [ ] **Step 1: Write the failing tests:** `sync_defaults_batch_and_wait` (`[sync]` empty → 5 and 60); `zero_batch_size_is_refused` (message contains `batch_size`); `zero_max_wait_is_refused`; `after_edit_false_still_loads_with_a_warning` (`after_edit = false` → `Ok`, `warnings()` equals the exact text); update the existing literal `SyncConfig { … after_edit: true … }` at `tests/config.rs:158`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test config` — expect FAIL.
- [ ] **Step 3: Implement** the fields, validation in `load`, and `warnings`. Callers of `after_edit` (`write.rs`, `cli/op.rs`, `cli/conflicts.rs`) keep today's behaviour until Task 6 rewires them: replace each `s.after_edit` check with `true`. Show `warnings()` on stderr in `tdo sync` (non-background) and `tdo status`, and as the TUI's first message at startup.
- [ ] **Step 4: Update `docs/config.md`:** remove the `after_edit` row; add `batch_size` and `max_wait_secs` rows with the spec §7.3 meanings; note that `after_edit` is ignored and `tdo sync off` keeps syncs manual.
- [ ] **Step 5: Run** `cargo test --workspace` — expect PASS (fix tests that set `after_edit`, e.g. `tdo/tests/write.rs`, `cli_op.rs`, `sync_e2e.rs`, to drop it; the `after_edit_*` and `SYNC_DELAY` tests are replaced in Task 6 — mark them `#[ignore = "replaced in Task 6"]` only if they cannot compile otherwise, and delete them in Task 6).
- [ ] **Step 6: Commit** `feat(config): add batch_size and max_wait_secs; retire after_edit`

### Task 4: The switch and the batching rule (core)

**Files:**
- Create: `crates/twydo-core/src/sync/switch.rs`, `crates/twydo-core/src/sync/batch.rs`
- Modify: `crates/twydo-core/src/sync/mod.rs` (modules; `SyncSummary.off: bool`), `crates/twydo-core/src/sync/run.rs` (`Skip::Off`)
- Test: `crates/twydo-core/tests/sync_batch.rs` (new), `crates/twydo-core/tests/sync_run.rs`

**Interfaces:**
- Produces:
  - `switch::is_off(cfg: &Config) -> io::Result<bool>` (creates nothing); `switch::set(cfg: &Config, off: bool) -> io::Result<()>` (binds the state dir; creates/removes `sync/off` durably).
  - `run::Skip::Off` — returned by `run` when `opts.background && is_off`, checked right after the `NotConfigured` check, before the run lock; writes nothing (no status, no `ended`).
  - `batch::Due` enum: `Nothing`, `Now`, `At(DateTime<Utc>)`.
  - `batch::decide(edits: &[DateTime<Utc>], last_attempt: Option<DateTime<Utc>>, next_attempt: Option<DateTime<Utc>>, size: u32, max_wait: Duration, now: DateTime<Utc>) -> Due` — pure.
  - `batch::due(cfg: &Config, now: DateTime<Utc>) -> io::Result<Due>` — `Nothing` without `[sync]`, when off, or with no state; else `decide` over `Journal::pending()` `created` times and `SyncStatus`.
  - `SyncSummary { …, pub off: bool }` filled by `summary`.

`decide` rule, in order: keep `edits` with `t >= last_attempt` (all if `None`); none → `Nothing`; deadline = `max(oldest + max_wait, next_attempt)` (ignore `next_attempt` if `None`); `count >= size` and `next_attempt` not in the future → `Now`; `now >= deadline` → `Now`; else `At(deadline)`.

- [ ] **Step 1: Write the failing tests** in `sync_batch.rs` (all with `decide`, `size = 5`, `max_wait = 60 s`, `now` fixed):
  - `no_edits_is_nothing`.
  - `one_edit_waits_until_oldest_plus_max_wait`: edit at `now-10s` → `At(now+50s)`.
  - `fifth_edit_is_due_now`: five edits within 5 s → `Now`.
  - `past_deadline_is_due_now` (Review Focus 2): one edit at `now-2h` → `Now`.
  - `edits_before_the_last_attempt_do_not_count` (Review Focus 3): ten edits at `now-1h`, `last_attempt = now-30m` → `Nothing`; plus one edit at `now-5s` → `At(now+55s)`.
  - `backoff_pushes_the_deadline` (Review Focus 4): one edit at `now-2m`, `next_attempt = now+10m` → `At(now+10m)`; six edits with the same backoff → `At(now+10m)` (not `Now`).
  - In `sync_run.rs`: `switched_off_background_run_is_skipped` (`switch::set(cfg, true)`, background → `Skip::Off`, fake not called — fake writes a marker file; `ended::read` is `None`); `switched_off_manual_run_still_runs`; `switch_survives_and_clears` (`set(true)`, `is_off` true; `set(false)`, false; `is_off` on a vdir without state is false and creates nothing).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_batch --test sync_run` — expect FAIL.
- [ ] **Step 3: Implement** `switch.rs`, `batch.rs`, `Skip::Off`, `SyncSummary.off`.
- [ ] **Step 4: Run** the same — expect PASS.
- [ ] **Step 5: Commit** `feat(sync): batching rule and automatic-sync switch`

### Task 5: The batch waiter

**Files:**
- Create: `crates/twydo-core/src/sync/waiter.rs`
- Modify: `crates/tdo/src/main.rs` (`Sync` gets hidden `--when-due` flag), `crates/tdo/src/cli/sync.rs` (`sync()` dispatches to the waiter; `background_command` gains a `when_due: bool` parameter)
- Test: `crates/twydo-core/tests/sync_waiter.rs` (new)

**Interfaces:**
- Consumes: `batch::due`, `switch::is_off`, `run::run`, `run::RunOptions`.
- Produces:
  - `pub trait Clock { fn now(&mut self) -> DateTime<Utc>; fn sleep_until(&mut self, t: DateTime<Utc>); }` with `pub struct SystemClock;` (wall clock; sleeps in slices of at most 5 s so a changed wall clock is noticed).
  - `pub enum WaiterExit { AlreadyRunning, NothingDue, SwitchedOff }`.
  - `pub fn wait_and_sync(cfg: &Config, clock: &mut dyn Clock, sync: &mut dyn FnMut() -> Result<RunReport, SyncError>) -> io::Result<WaiterExit>` — the loop of spec §7.2 steps 2–4, lock file `<vdir-state>/sync/waiter.lock` taken with `try_lock` (reuse the `RunLock` pattern from `run.rs`; make a shared `pub(crate)` helper rather than copying). The exit re-check (step 4) happens after unlocking: if `due` is not `Nothing`, try the lock again and continue the loop; if another waiter has it, return `AlreadyRunning`.
  - `tdo sync --background --when-due` calls `wait_and_sync(&cfg, &mut SystemClock, &mut || run(&cfg, &RunOptions { background: true, ..Default::default() }, Utc::now()))` and exits 0.

- [ ] **Step 1: Write the failing tests** with a fake `Clock` that records `sleep_until` targets and advances `now` to them, and a `sync` closure that counts calls and (to simulate a real sync) marks every pending journal entry synced and sets `status.last_attempt`:
  - `nothing_unsynced_exits_without_sleeping` → `NothingDue`, no sleeps, no syncs.
  - `sleeps_to_the_deadline_then_syncs_once` → one sleep to `oldest + 60 s`, one sync, then `NothingDue`.
  - `a_second_waiter_exits_at_once` → hold the lock in the test, call → `AlreadyRunning`.
  - `switched_off_while_sleeping_exits` (Review Focus 5) → the fake clock's `sleep_until` calls `switch::set(cfg, true)`; result `SwitchedOff`, zero syncs.
  - `timer_synced_meanwhile_exits_without_syncing` → `sleep_until` marks entries synced; zero syncs, `NothingDue`.
  - `edit_during_its_sync_gets_its_own_deadline` → the `sync` closure applies one new entry after confirming the old ones; expect a second sleep and second sync.
  - `edit_landing_at_exit_is_picked_up` → inject the edit from a test-only hook between unlock and re-check (expose `wait_and_sync_with(cfg, clock, sync, after_unlock: &mut dyn FnMut())`, with `wait_and_sync` passing a no-op); expect a further sync.
  - `backoff_is_honoured` (Review Focus 4) → `status.next_attempt = now+10m`: first sleep target is `now+10m`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_waiter` — expect FAIL.
- [ ] **Step 3: Implement** `waiter.rs`, the `--when-due` flag (hidden, requires `--background`), and `background_command(exe, config, vdir_flag, when_due)` adding `--when-due`. Spawn every detached background command with `CommandExt::process_group(0)`, so closing the terminal does not kill the waiter.
- [ ] **Step 4: Run** `cargo test -p twydo-core --test sync_waiter && cargo test -p tdo` — expect PASS.
- [ ] **Step 5: Commit** `feat(sync): batch waiter process (tdo sync --background --when-due)`

### Task 6: Wire the trigger into every writer; `:sync on|off`, `tdo sync on|off`, status line

**Files:**
- Modify: `crates/twydo-core/src/sync/batch.rs` (`after_write`), `crates/tdo/src/write.rs` (remove `SYNC_DELAY`, `sync_due`, `sync_soon`, `sync_if_quiet`, `flush_sync`; `SyncTrigger` becomes `Box<dyn FnMut(&Config, Trigger)>`), `crates/tdo/src/term.rs` (drop the quit flush and `sync_if_quiet` call; resolve path uses the trigger; `Outcome::SyncSwitch`), `crates/tdo/src/app.rs` (`:sync on`, `:sync off`), `crates/tdo/src/cli/op.rs`, `crates/tdo/src/cli/conflicts.rs`, `crates/tdo/src/cli/sync.rs` + `main.rs` (`tdo sync on|off`, `tdo status` prints `automatic sync: on|off`), `crates/tdo/src/ui/status.rs` (`sync off` segment)
- Modify docs: `docs/sync.md` (triggers: batching, waiter, switch, event; replace the `after_edit` paragraph at line ~61), `docs/tui.md` (status line `sync off`, `:sync on|off`, the `after_edit` sentence at line ~292), `docs/architecture.md` (sync section: `ended`, `off`, `waiter.lock`), `AGENTS.md` progress note
- Test: `crates/tdo/tests/write.rs`, `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/cli_op.rs`, `crates/tdo/tests/cli.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `batch::due`, `switch::{is_off, set}`, `background_command(…, when_due)`.
- Produces:
  - `pub enum Trigger { SyncNow, Waiter }` in `twydo_core::sync::batch`.
  - `pub fn after_write(cfg: &Config, now: DateTime<Utc>) -> Option<Trigger>` — `Due::Now` → `SyncNow`; `Due::At(_)` → `Waiter`; `Nothing` → `None`.
  - `tdo::cli::sync::spawn(config: &Config, vdir_flag: Option<&Path>, t: Trigger)` replacing `spawn_background` at write sites (`spawn_background` stays for the TUI startup sync).
  - `Outcome::SyncSwitch(bool)` (true = on) from `:sync on` / `:sync off`.

- [ ] **Step 1: Write the failing tests:**
  - `write.rs`: replace the `after_edit_*`, `each_save_pushes…`, `sync_soon…` tests with: `first_save_starts_the_waiter` (trigger called once with `Waiter`); `fifth_save_syncs_now` (`batch_size = 5`, five saves → last call `SyncNow`); `refused_save_triggers_nothing`; `no_sync_section_triggers_nothing`; `switched_off_triggers_nothing`. Delete `after_edit_sync_starts_after_the_stop_request_is_cleared`'s `flush_sync` use (keep its stop assertions; the trigger now fires inside `stop_and_run`).
  - `app_keys.rs`: `colon_sync_off_and_on` → `:sync off` returns `Outcome::SyncSwitch(false)`, `:sync on` → `SyncSwitch(true)`; `:sync bogus` → message `usage: :sync [on|off]`.
  - `cli.rs`: `tdo sync off` then `tdo status` prints `automatic sync: off`; `tdo sync on` prints `automatic sync on`; both exit 0; without `[sync]` → exit 2 with the not-configured message.
  - `cli_op.rs`: replace `after_edit_false_does_not_trigger` with `op_starts_the_waiter_after_a_write` (the fake vdirsyncer is not called within 1 s of one `tdo op`, i.e. no immediate sync) and `op_with_sync_off_starts_nothing`.
  - `ui_render.rs`: status line contains `sync off` while off, alongside `N pending`.
- [ ] **Step 2: Run** `cargo test -p tdo` — expect FAIL.
- [ ] **Step 3: Implement.** `Writer::run`/`stop_and_run` call `after_write` on success and invoke the trigger. `term.rs` `SyncSwitch(on)`: `switch::set`; on `on` with `batch::due(...) != Nothing` spawn `SyncNow`; message `automatic sync on` / `automatic sync off`; `refresh_sync`. `tdo sync on` does the same (detached sync). Status line: when `summary.off`, prepend a warning-styled `sync off` segment and show it even when `pending == 0`.
- [ ] **Step 4: Update the docs** listed above; run `python3 tools/check_links.py` — expect no output and exit 0.
- [ ] **Step 5: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check` — expect PASS.
- [ ] **Step 6: Commit** `feat: batched syncs after edits and an automatic-sync switch`

### Task 7: End-to-end check and close-out

**Files:**
- Modify: `crates/tdo/tests/sync_e2e.rs` (drop `after_edit = false` from its config; it now uses `tdo sync off` in setup if it needs manual syncs)
- Modify: `AGENTS.md` (progress: plan 1 of the write queue done; next plan 2)

- [ ] **Step 1: Run** `cargo test --workspace` — expect PASS.
- [ ] **Step 2: Manual check with the synthetic test setup only, after asking the user (never production):** `tdo sync off`, edit two tasks in the TUI → status line `sync off · 2 pending`, no sync; `:sync on` → a sync starts at once and `pending` clears; edit one task → no sync for ~60 s, then one sync and the TUI reloads without key presses. Record the result in the PR description.
- [ ] **Step 3: Commit** `docs: record write-queue plan 1 progress` and open the PR referencing #38 (partial) and closing #30.
