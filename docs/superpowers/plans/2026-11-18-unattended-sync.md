# Unattended Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply lists emptied on the server automatically when nothing local-only can be lost,
log sync state changes from the daemon, and show sync trouble on the phone.

**Architecture:** The sync runner's `auto_force` (core) learns a second, guarded case for
server-side emptying and reports which lists it applied and which it held back and why. The
daemon turns each run into change-only stderr lines (a small stateful logger in `Shared`) and
adds a `sync` summary to `views.get`, built by a pure function from the cached `StatusView`.
The web app renders that summary as a persistent warning strip.

**Tech Stack:** Rust workspace (`twydo-core`, `twydo-daemon`), vdirsyncer driven as a child
process, Preact + TypeScript web app (`web/`) with types generated from `schema/tdo.openrpc.json`.

**Spec:** `docs/superpowers/specs/2026-11-18-unattended-sync-design.md`

## Global Constraints

- Server-side emptying is applied everywhere (desktop and container); no setting.
- Logs name lists by collection id, never display name or task title. Log lines start
  `tdo daemon: ` (the logger returns lines without it; the caller adds it).
- Logs print on change only: identical retries stay quiet.
- The phone is inform-only: no new gateway methods, no change to the gateway allowlists.
- The phone may show list display names; never task titles beyond what views already show.
- Lists emptied **here** keep today's auto-force rule unchanged.
- Commit messages pass commitlint (`type(scope): subject`, lowercase subject, ≤100 chars).
  No agent attribution lines.
- Cargo target never under `/tmp` (use the default or `~/.cache`).
- Changed behaviour updates its doc page; `python3 tools/check_links.py` passes.

## Review Focus

1. **Local edit made after the emptying is reported, before the forced pass.** A pending
   journal write in the list must hold it back (Task 1, `held_when_queued_edit_in_list`).
2. **New local task never uploaded** (a file in the list the manifest does not record) must
   hold it back, not be deleted (Task 1, `held_when_local_only_file`).
3. **No manifest entry for the list** (first sync after an upgrade, or after a failed run
   dropped it) must hold it back (Task 1, `held_without_manifest_record`).
4. **A long vdirsyncer error in `last_error`** must not make the phone strip unreadable: the
   detail is capped (Task 3, `detail_is_capped`).
5. **Daemon restart while failing** logs the current failure once, not zero times
   (Task 2, `fresh_logger_reports_current_failure`).

---

### Task 1: Guarded auto-apply of server-side emptying (core)

**Files:**
- Modify: `crates/twydo-core/src/sync/manifest.rs` (add `covers_local`)
- Modify: `crates/twydo-core/src/sync/run.rs` (`auto_force`, its call site ~line 400,
  `RunReport`)
- Modify: `crates/twydo-core/src/sync/report.rs` (`messages` notes)
- Create: `crates/twydo-core/tests/data/vdirsyncer/emptied-remote.txt`,
  `crates/twydo-core/tests/data/vdirsyncer/emptied-remote-all.txt`
- Test: `crates/twydo-core/tests/sync_run.rs`, `crates/twydo-core/tests/sync_output.rs`,
  unit tests in `manifest.rs`
- Docs: `docs/sync.md` § "Emptied lists"

**Interfaces:**
- Produces (used by Task 2):
  - `RunReport.auto_forced_remote: Vec<String>`: the subset of `auto_forced` applied because
    the server emptied them. `auto_forced` keeps holding both kinds.
  - `RunReport.held_emptied: BTreeMap<String, HeldEmptied>`: emptied lists (either side) this
    run did not force, keyed by collection id.
  - `pub struct HeldEmptied { pub remote: bool, pub reason: HeldReason }`
  - `pub enum HeldReason { NotEmptiedByTwydo, NoRecord, LocalOnlyFiles, QueuedEdits,
    Conflict, EveryListEmptied }` with `Display`: `"not emptied by Twydo"`,
    `"no record of the last sync"`, `"tasks here that the server never had"`,
    `"edits here not yet synced"`, `"unresolved conflict"`,
    `"every list came back empty"`. All in `twydo_core::sync::run`, derive
    `Debug, Clone, PartialEq, Eq`.
  - `Manifest::covers_local(&self, vdir: &Path, coll: &str) -> io::Result<Option<bool>>`:
    `None` when the manifest has no entry for `coll`; else whether every `.ics` file now in
    `vdir/coll` (same filter as `refresh`: plain files, not dot-files) is listed for it.

- [ ] **Step 1: Add fixtures.** `emptied-remote.txt` is `emptied-local.txt` with
  `twydo_local/c1` replaced by `twydo_remote/c1` (both places). `emptied-remote-all.txt` adds
  the same `error:` line for `Twydo test` after its `Syncing` line, so both collections
  report emptied on `twydo_remote`.

- [ ] **Step 2: Write failing tests** in `sync_run.rs`, modelled on
  `queued_final_task_deletion_is_automatically_sent_on_the_rerun`. Each: `seed_tasks`, a first
  `run(..., &manual(), now())` with `clean.txt` to write the manifest, then the fake prints the
  emptied fixture unless `$2 = --force-delete` (then `clean.txt`), recording calls.
  - `server_emptied_list_is_applied`: second run → `report.auto_forced == ["c1"]`,
    `report.auto_forced_remote == ["c1"]`, `collections["c1"] == Ok`, calls contain
    `--force-delete twydo/c1`, `held_emptied` empty.
  - `held_when_local_only_file`: before the second run add `c1/new.ics` (copy of a seeded
    file under a new name) → no `--force-delete` call, `collections["c1"] == EmptiedRemote`,
    `held_emptied["c1"] == HeldEmptied { remote: true, reason: LocalOnlyFiles }`.
  - `held_when_queued_edit_in_list`: before the second run apply a title edit to `QA` through
    the store wrapper (leaves a pending journal entry) → `reason: QueuedEdits`, not forced.
  - `held_without_manifest_record`: skip the first run → `reason: NoRecord`.
  - `held_when_list_has_conflict`: seed an unresolved conflict for `c1` in the conflict store
    → `reason: Conflict`.
  - `held_when_every_list_emptied`: fake prints `emptied-remote-all.txt` → both held with
    `EveryListEmptied`, no forced call.
  - `local_emptying_not_by_twydo_is_held`: delete the `c1` files by hand, fake prints
    `emptied-local.txt` → `HeldEmptied { remote: false, reason: NotEmptiedByTwydo }`.
  - Unit tests in `manifest.rs`: `covers_local` returns `None` with no entry, `Some(true)` for
    a subset of the listed files, `Some(false)` with an extra `.ics`, and ignores dot-files.
  - In `sync_output.rs` (next to the existing `auto_forced: vec!["c3".into()]` case):
    `messages` with `auto_forced = ["c3"]`, `auto_forced_remote = ["c3"]` gives the note
    `applied the list "<name>" emptied on the server`; a local one keeps
    `applied the emptied list "<name>" on the server`.

- [ ] **Step 3: Run them to see them fail.**
  Run: `cargo test -p twydo-core --test sync_run --test sync_output && cargo test -p twydo-core --lib manifest`
  Expected: compile errors for the missing fields and method, then failures.

- [ ] **Step 4: Implement.** `auto_force` gains `vdir: &Path` and returns
  `struct AutoForce { local: Vec<String>, remote: Vec<String>, held: BTreeMap<String, HeldEmptied> }`.
  For each emptied collection not in `explicit`:
  - local side: today's rules; when they fail, held with `Conflict` if it has an unresolved
    conflict, else `NotEmptiedByTwydo`.
  - remote side, checked in this order: `EveryListEmptied` when the run's `coll_states` has
    ≥2 collections and all are `EmptiedRemote`; `Conflict`; `QueuedEdits` when any write in
    `pending` has a path starting with `"<coll>/"`; `NoRecord` when `covers_local` is `None`;
    `LocalOnlyFiles` when it is `Some(false)`; else forced.

  The call site reruns `local ∪ remote` in the one existing second pass, extends `forced` and
  `auto_forced` with both, `auto_forced_remote` with `remote`, and sets `held_emptied`. A held
  list whose second pass did not happen (the first pass did not exit) is still reported held.
  `messages` picks the note wording by membership in `auto_forced_remote`.

- [ ] **Step 5: Run the tests.** Same command as Step 3, then `cargo test -p twydo-core` and
  `cargo test -p tdo --test sync_e2e`. Expected: all pass.

- [ ] **Step 6: Update `docs/sync.md`** § "Emptied lists": add the server-side bullet with the
  five checks and the new note, and change "Anything else is never forced" to cover only what
  fails them. Run `python3 tools/check_links.py`.

- [ ] **Step 7: Commit.**
  `git commit -m "feat(sync): apply lists emptied on the server when nothing local is lost"`
  with body `Closes #142.`

### Task 2: Daemon logs sync state changes (#141)

**Files:**
- Create: `crates/twydo-daemon/src/server/synclog.rs`
- Modify: `crates/twydo-daemon/src/server/mod.rs` (`pub mod synclog;`, a
  `sync_log: Mutex<SyncLog>` field in `Shared`, default-initialised)
- Modify: `crates/twydo-daemon/src/server/syncjob.rs` (`run_sync`)
- Test: `crates/twydo-daemon/tests/synclog.rs`
- Docs: `docs/gateway.md` (logs section, near "writes all its log lines ... standard error")

**Interfaces:**
- Consumes: `RunReport.auto_forced`, `auto_forced_remote`, `held_emptied`, `skipped` (Task 1);
  `twydo_core::sync::status::SyncStatus`.
- Produces: `#[derive(Default)] pub struct SyncLog` with
  `pub fn lines(&mut self, report: Option<&RunReport>, status: &SyncStatus, now: DateTime<Utc>) -> Vec<String>`.
  `report` is `None` when `run_with` returned `Err`.

- [ ] **Step 1: Write failing tests** in `tests/synclog.rs`, building `SyncStatus` and
  `RunReport` values directly:
  - `first_success_logs_list_count`: status with 3 collections `Ok`, `last_success` set →
    `["sync ok (3 lists)"]`; calling again with the same input → `[]`.
  - `failure_logs_once_per_error_text`: `last_error = "boom"`, `failures = 2`,
    `next_attempt = now + 120 s` → `["sync failed (attempt 2, next in 120s): boom"]`; same
    error with `failures = 3` → `[]`; `last_error = "other"` → one line again. Without
    `next_attempt` the line is `sync failed (attempt 2): boom`.
  - `recovery_logs_failure_count`: after a failure with `failures = 4`, a status with no error
    → `["sync ok again after 4 failures"]`.
  - `fresh_logger_reports_current_failure`: a new `SyncLog` given a failing status logs it.
  - `discovery_error_logged_on_change`: `discovery_error = Some("x")` →
    `list discovery failed: x`, once.
  - `applied_emptyings_logged_every_time`: `auto_forced = ["a", "b"]`,
    `auto_forced_remote = ["a"]` → `applied list a emptied on the server`,
    `applied list b emptied here on the server`; the same report again logs both again.
  - `held_emptying_logged_when_new`: `held_emptied = {"a": remote, QueuedEdits}` →
    `list a emptied on the server; not applied (edits here not yet synced)`; again → `[]`;
    a report without it, then with it again → logged again. Local side wording:
    `list a emptied here; not applied (not emptied by Twydo)`.
  - `skipped_run_logs_nothing`: `report.skipped = Some(..)` → `[]` and state unchanged.

- [ ] **Step 2: Run to see them fail.**
  Run: `cargo test -p twydo-daemon --test synclog`  Expected: does not compile.

- [ ] **Step 3: Implement `SyncLog`.** State: whether a success was logged since start, the
  last logged `last_error`, the `failures` count seen with it, the last logged
  `discovery_error`, and the set of held collection ids last seen. Order of lines: failure or
  success line, discovery line, applied lines, held lines. Seconds are
  `(next_attempt - now).num_seconds().max(0)`.

- [ ] **Step 4: Wire it in `run_sync`.** After `run_with`, load
  `SyncStatus::load(<state dir>)` (on error print `tdo daemon: cannot read sync status: {e}`
  and skip), call `shared.sync_log.lock().unwrap().lines(...)`, and `eprintln!` each as
  `tdo daemon: {line}`.

- [ ] **Step 5: Run the tests.** `cargo test -p twydo-daemon`. Expected: all pass.

- [ ] **Step 6: Document** in `docs/gateway.md` the lines a container log shows (one example
  of each), and note the same lines land in `daemon-<socket-id>.log`. Run
  `python3 tools/check_links.py`.

- [ ] **Step 7: Commit.** `git commit -m "feat(daemon): log sync state changes to stderr"`
  with body `Closes #141.`

### Task 3: `views.get` carries a sync summary

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/views.rs` (`SyncSummary`, field on
  `ViewsGetResult`)
- Create: `crates/twydo-daemon/src/server/syncsummary.rs`
- Modify: `crates/twydo-daemon/src/server/mod.rs`, `crates/twydo-daemon/src/server/views.rs`
- Modify: `schema/tdo.openrpc.json` (bless)
- Test: `crates/twydo-daemon/tests/syncsummary.rs`, `crates/twydo-daemon/tests/views.rs`
- Docs: `docs/protocol.md` (`views.get` bullet)

**Interfaces:**
- Consumes: `StatusView`/`SyncView` from `shared.publisher.status()`;
  `RunReport` is not used here. Refused emptyings are read from
  `SyncView.collections` (`WireCollState::EmptiedRemote` / `EmptiedLocal`).
- Produces (used by Task 4 through the generated TS types):
  ```rust
  pub struct SyncSummary {
      pub attention: bool,
      pub headline: String,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub detail: Option<String>,
      pub last_success: Option<DateTime<Utc>>,
  }
  // ViewsGetResult gains:
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sync: Option<SyncSummary>,
  ```
  and `pub fn summary(sync: &SyncView, conflicts: usize, now: DateTime<FixedOffset>, list_name: &dyn Fn(&str) -> String) -> SyncSummary`.

- [ ] **Step 1: Write failing tests** in `tests/syncsummary.rs` (now = 2026-11-18 14:00 +01:00):
  - `failing_sync`: `last_error = "Cannot reach the server"`, `next_attempt` 13:10 UTC →
    `attention`, headline `Sync failing`, detail `Cannot reach the server. Next try 14:10.`
  - `discovery_failure_in_last_error`: `last_error` starting `list discovery failed;` →
    headline `List discovery failed`, detail is the error text.
  - `discovery_error_only`: no `last_error`, `discovery_error = "x"` → headline
    `List discovery failed`, detail `x`.
  - `emptied_not_applied`: `collections = {"c1": EmptiedRemote}`, name `Inbox` → headline
    `"Inbox" was emptied on the server`, detail
    `Not applied automatically. Run: tdo sync --force-delete "Inbox"`. Local side: `here`.
    Two lists → headline `2 lists were emptied`, detail the two `Run:` commands joined by
    `; `.
  - `conflicts`: `conflicts = 2` → headline `2 conflicts`, detail `Resolve them in tdo.`
    (`1 conflict` for one).
  - `sync_off`: `auto = false` → headline `Sync is off`, detail `Turn it on in tdo.`
  - Priority: failing beats discovery beats emptied beats conflicts beats off; only the
    first applies.
  - `all_well`: `last_success` 5 minutes ago → `attention == false`, headline
    `Synced 5m ago`, no detail; never synced → `Not synced yet`. Use the TUI's `ago` buckets
    (`just now`, `Nm ago`, `Nh ago`, `Nd ago`).
  - `detail_is_capped`: a 1000-character error → detail is at most 300 characters and ends
    with `…`.
  - In `tests/views.rs`: `sync_absent_without_sync_config` (existing fixture, `sync` is
    `None`); `sync_summary_reports_failure`: start with `fx.with_sync(...)` and a pre-written
    `<state>/sync/status.json` holding `last_error` and `failures: 1` → `sync.attention`.

- [ ] **Step 2: Run to see them fail.**
  Run: `cargo test -p twydo-daemon --test syncsummary --test views`

- [ ] **Step 3: Implement** `SyncSummary`, `summary`, and in `views::get` set
  `sync: shared.publisher.status().and_then(|s| s.sync).map(|v| summary(&v, s.conflicts, now.fixed_offset(), &list_name))`.
  Move the TUI's `ago` buckets into a shared helper only if it stays trivial; otherwise
  duplicate the four-arm match.

- [ ] **Step 4: Bless the schema and run the tests.**
  `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, then `cargo test -p twydo-daemon`
  and `cargo test -p twydo-gateway`. Expected: all pass, `schema/tdo.openrpc.json` changed.

- [ ] **Step 5: Document** the `sync` field in the `views.get` bullet of `docs/protocol.md`.
  Run `python3 tools/check_links.py`.

- [ ] **Step 6: Commit.** `git commit -m "feat(daemon): add a sync summary to views.get"`

### Task 4: Warning strip in the web app

**Files:**
- Create: `web/src/components/SyncWarning.tsx`
- Modify: `web/src/App.tsx` (render under the `large-title` header), `web/src/styles.css`
- Test: `web/src/components/App.test.tsx`
- Docs: `docs/gateway.md` (what the phone shows)

**Interfaces:**
- Consumes: `ViewsGetResult.sync?: SyncSummary` from `web/src/gen/protocol` (regenerated by
  `npm run gen` from the Task 3 schema).
- Produces: `export function SyncWarning({ sync }: { sync: SyncSummary }): JSX.Element`.

- [ ] **Step 1: Write failing tests** in `App.test.tsx`:
  - `shows sync warning when attention`: a view with `sync: { attention: true, headline:
    "Sync failing", detail: "Cannot reach the server. Next try 14:10.", last_success: null }`
    renders both texts in an element with `role="alert"`.
  - `no warning when all is well`: `attention: false` → no `role="alert"`, and the headline
    text is not rendered.
  - `warning clears on refetch`: a refetched view without attention removes it.

- [ ] **Step 2: Run to see them fail.** `cd web && npm run check`

- [ ] **Step 3: Implement.** `SyncWarning` is a `div.sync-warning` with `role="alert"`, the
  headline in `<strong>`, the detail in a `<p>`; no close button. App renders it only when
  `view?.sync?.attention`. Style with the existing warning colour tokens in `styles.css`.

- [ ] **Step 4: Run the checks.** `cd web && npm run check && npm run build`. Expected: pass.

- [ ] **Step 5: Document** the warning strip in `docs/gateway.md` (what triggers it, that it
  clears itself, that fixing happens in `tdo`). Run `python3 tools/check_links.py`.

- [ ] **Step 6: Commit.** `git commit -m "feat(web): show a sync warning on the phone"`

### Final check

- [ ] `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- [ ] `cd web && npm run check`
- [ ] `npx --yes -p @commitlint/cli -p @commitlint/config-conventional commitlint --from=origin/main --to=HEAD`
