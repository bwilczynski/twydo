# Twydo Plan 3: Sync runner and conflicts — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sync the vdir with the NAS automatically and safely. `tdo sync` runs vdirsyncer
(and metasync) serialized with Twydo's writes. It confirms and prunes journal entries once the
server has them, and keeps both versions of every conflict in a recoverable store until the user
picks one. A systemd timer or launchd agent runs it every minute, and `tdo op` triggers it after
each write. The TUI shows sync state and resolves conflicts.

**Architecture:** `twydo-core` gains a `sync` module and a `conflicts` store. The runner spawns
`vdirsyncer sync <pair>` under the shared vdir lock and parses its stderr. It then takes the
exclusive lock to reconcile the result with the pending journal entries. vdirsyncer's
`conflict_resolution = ["command", …]` hook calls `tdo conflicts hook`. The hook records both
versions and applies a choice once the user has made one. Delete-versus-edit is a case vdirsyncer
settles silently, so the runner detects it from the journal and vdirsyncer's transfer lines.
Twydo still has no network code.

**Tech Stack:** existing workspace (Rust 2024, chrono, serde/serde_json/toml, thiserror, clap,
ratatui, notify). No new crates. Use `std::fs::File::try_lock` for the run lock, and
`std::process` with reader threads plus a `try_wait` poll for the timeout.

**Spec:** `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` §5 (storage and sync
split; conflicts), §6 (recovery: "retain recovery information until sync is confirmed"), §8
(`:sync`, `:conflicts`), §9 (failures/conflicts, setup), §10 step 5. Journal format:
`docs/journal.md`. Session evidence: `docs/reference/phase0/plan4-baseline.md` (Observations, Results).

## Scope

In: `[sync]` config, the runner (`tdo sync`, `tdo status`), journal confirmation and pruning,
the conflict store, the conflict hook, delete-versus-edit detection, journal entries stuck after
a conflict, "completely emptied" handling, backoff, timeout, the after-edit trigger from
`tdo op`, TUI status, `:sync`, `:conflicts`, systemd/launchd units, and one session on the test
account (Task 13).

Out: the notifier and its reading of conflicted tasks' triggers (Plan 2). TUI write keys, and
running the after-edit trigger from them (Plan 5, via the function from Task 10). List creation
through the runner (Plan 5). Undo after a remote edit keeps Plan 4's behaviour: it is refused as
`Stale` and never overwrites. Twydo cannot fix the iPhone's transient "invalid task reference"
error after vdirsyncer deletes without an etag. Task 13 records whether it still happens.

## Facts this plan builds on (probed 2026-11-01, vdirsyncer 0.21.0, two filesystem storages)

These come from the local probe, not from guesses. Task 3 turns its output into test fixtures.
Task 13 repeats the checks against Synology and the iPhone.

- F1. vdirsyncer writes all progress and errors to **stderr**. It exits 1 if any collection had an error, otherwise 0. Items are named by **UID**, not file name.
- F2. With `conflict_resolution = ["command", CMD, ARGS…]`, vdirsyncer runs `CMD ARGS… <tmp>/<storage a name> <tmp>/<storage b name>`. The files hold each side's raw item. If they are byte-equal when the command exits 0, that content is written to both sides (`Copying (updating) item …`). If they differ, vdirsyncer logs `critical: The two files are not completely equal.`, leaves the item alone, still syncs the other items of the collection, and exits 1.
- F3. A hook that exits non-zero gives `error: Unknown error occurred for <pair>/<coll>: Command '[…]' returned non-zero exit status N.`. Both sides stay untouched.
- F4. With no `conflict_resolution`, a conflict gives `error: <pair>/<coll>: One item changed on both sides. …` and continuation lines (`error: See also …`, `error: Item ID: <uid>`, `error: Item href on side A: …`). Other items still sync.
- F5. Both sides changed to **identical** bytes: `...same content on both sides.`, exit 0, and the hook is not called.
- F6. **Delete versus edit is settled silently, exit 0, no hook**, in both directions. A local delete with a remote edit restores the edit locally (`Copying (uploading) item <uid> to <local>/<coll>`). A remote delete with a local edit re-uploads the edit (`Copying (uploading) item <uid> to <remote>/<coll>`). So a deletion can be lost without any error (spec §5 forbids that silently).
- F7. An emptied side: `error: <pair>/<coll>: Storage "<storage>/<coll>" was completely emptied. If you want to delete ALL entries on BOTH sides,then use \`vdirsyncer sync --force-delete <pair>/<coll>\`. …`, exit 1. `--force-delete <pair>/<coll>` then runs a normal sync for that collection.
- F8. Other lines: `Syncing <pair>/<coll>`, `Copying (updating) item <uid> to <storage>/<coll>`, `Deleting item <uid> from <storage>/<coll>`, `Doing conflict resolution for item <uid>...`, `error: Use \`-vdebug\` to see the full traceback.`.

## Global Constraints

- **Conflicts (spec §5):** never configure `a wins`/`b wins`. Keep both versions of a conflicted resource outside the vdir until the user chooses. Never silently discard a completion, recurrence advancement or deletion. Other resources keep syncing.
- **Recovery (spec §6):** finish journal recovery before running vdirsyncer. Keep a journal entry until a sync confirms the server holds its result. vdirsyncer runs under the **shared** vdir lock (readers continue; writers wait). Reconciliation after the sync runs under the **exclusive** lock.
- **Reminders survive sync failures (spec §5):** the runner never holds the exclusive lock while waiting on the network. vdirsyncer is killed after `timeout_secs`.
- **Serialized runs (spec §5):** at most one runner per vdir (`<state>/sync/run.lock`).
- **Lock order:** run lock → vdir lock → conflicts lock. Never take them in another order.
- **Auxiliary formats are inspectable (spec §6):** status and conflict records are pretty JSON. Conflict versions are plain `.ics` files.
- **No network code in Twydo:** everything goes through the configured vdirsyncer command.
- Tests never read the clock: the runner takes `now: DateTime<Utc>`. Timeouts use `Instant`.
- Tests that need vdirsyncer check `vdirsyncer --version` first. If it is missing they print `skipping: vdirsyncer not installed` and return.
- The test setup sets `XDG_CONFIG_HOME` for `tdo` only. That hides vdirsyncer's own config, so `[sync] vdirsyncer_config` must be passed to vdirsyncer as `VDIRSYNCER_CONFIG`.
- Ask the user before touching their vdirsyncer config, the NAS or the iPhone (Task 13 only, test account only).
- No personal data in the repo. vdirsyncer output captured from CalDAV is anonymised (URL, user name, collection names) before committing.
- Commit messages carry no agent attribution (user's global instructions).

## Review Focus

1. **A phone edit silently undoes a Twydo deletion, or the reverse** (F6, exit 0). The runner records a conflict whose deleted side is kept, and the user decides. Tests → Task 7 (`local_delete_vs_remote_edit_becomes_conflict`, `remote_delete_vs_local_edit_becomes_conflict`).
2. **The server reports a list as empty** (a server glitch, or the phone deleted its last task). Never `--force-delete` automatically when the remote side emptied; local tasks stay. Test → Task 8 (`remote_emptied_is_never_forced`).
3. **vdirsyncer hangs** (NAS asleep, network half-up) while the runner holds the shared lock. It is killed after `timeout_secs`, counted as a transient failure with backoff, and writers can proceed. Test → Task 6 (`hung_command_is_killed_after_timeout`).
4. **The timer, the after-edit trigger and `:sync` fire together.** One vdirsyncer runs at a time. A background run that finds the lock busy asks the running one for a rerun and exits 0. Test → Task 6 (`busy_background_run_requests_rerun`).
5. **vdirsyncer output Twydo does not recognise** (a new vdirsyncer version, a network error). Nothing is confirmed or pruned, and the failure is shown. Test → Task 3 (`unknown_error_line_is_unattributed`) and Task 7 (`unattributed_error_confirms_nothing`).

---

## File structure

```
crates/twydo-core/src/
  config.rs            + SyncConfig, Config.sync
  store/journal.rs     + Entry.synced, Journal::{pending, mark_synced, prune}
  store/mod.rs         + Store::restore (kind "resolve")
  conflicts.rs         new: ConflictStore, Conflict, ConflictKind, Side
  sync/mod.rs          new: pub mod; SyncError; summary()
  sync/output.rs       new: parse vdirsyncer stderr → SyncOutput
  sync/status.rs       new: SyncStatus (status.json), backoff
  sync/hook.rs         new: conflict_hook (called by vdirsyncer), hook log
  sync/run.rs          new: run(): locks, recovery, vdirsyncer, emptied, metasync, status
  sync/reconcile.rs    new: after-sync confirmation, delete-vs-edit, held-conflict bookkeeping, pruning
  sync/resolve.rs      new: resolve(conflict, side)
crates/twydo-core/tests/data/vdirsyncer/*.txt   new: stderr fixtures (synthetic)
crates/tdo/src/cli/sync.rs        new: tdo sync, tdo status, spawn_background
crates/tdo/src/cli/conflicts.rs   new: tdo conflicts [list | resolve | hook]
crates/tdo/src/cli/op.rs          + after-edit trigger
crates/tdo/src/app.rs, term.rs, ui/{status,tasks,details}.rs, ui/conflicts.rs (new)
crates/tdo/tests/sync_e2e.rs      new: real vdirsyncer between two filesystem storages
contrib/systemd/twydo-sync.{service,timer}, contrib/launchd/app.twydo.sync.plist   new
contrib/vdirsyncer/config.example  + conflict hook line
docs/sync.md new; docs/config.md, docs/journal.md updated; docs/reference/phase0/plan3-sync.md new (Task 13)
```

State layout added under `<state>` (= `<state_dir>/vdirs/<id>/`, `docs/journal.md`):

```
sync/run.lock        runner lock (exclusive, try_lock in background mode)
sync/requested       empty file: "run once more when you finish"
sync/status.json     SyncStatus
sync/hook.log        JSON lines written by the hook during one run (truncated at run start)
conflicts/.lock      conflict store lock
conflicts/<id>/conflict.json, local.ics, remote.ics   (a missing .ics = that side deleted it)
```

---

### Task 1: `[sync]` configuration

**Files:**
- Modify: `crates/twydo-core/src/config.rs`, `docs/config.md`
- Test: `crates/twydo-core/tests/config.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Deserialize, Clone, Debug, PartialEq)] #[serde(deny_unknown_fields)]
  pub struct SyncConfig {
      pub pair: String,                       // default "twydo"
      pub local_storage: String,              // default "twydo_local" (the vdir side of the pair)
      pub command: String,                    // default "vdirsyncer"
      pub vdirsyncer_config: Option<PathBuf>, // tilde-expanded; passed as VDIRSYNCER_CONFIG
      pub metasync: bool,                     // default true
      pub after_edit: bool,                   // default true
      pub timeout_secs: u64,                  // default 300
      pub max_backoff_secs: u64,              // default 1800
      pub retention_days: u32,                // default 30
  }
  pub struct Config { …, pub sync: Option<SyncConfig> }   // no [sync] table = sync off
  ```
- [ ] **Step 1: Write the failing tests:** `sync_absent_means_off` (`cfg.sync == None`); `sync_defaults` (an empty `[sync]` table gives exactly the defaults above); `sync_vdirsyncer_config_expands_tilde`; `sync_unknown_key_is_error` (the message names the key).
- [ ] **Step 2:** Run `cargo test -p twydo-core --test config`. Expected: FAIL.
- [ ] **Step 3:** Implement. Add a `[sync]` table to `docs/config.md`. Say that a missing table turns sync off. Say that `vdirsyncer_config` is needed when `XDG_CONFIG_HOME` is set for `tdo` only.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(config): [sync] section for the sync runner`.

### Task 2: Journal confirmation and pruning

**Files:**
- Modify: `crates/twydo-core/src/store/journal.rs`, `crates/tdo/src/cli/op.rs` (`tdo op journal` column), `docs/journal.md` (Retention)
- Test: `crates/twydo-core/tests/journal.rs`, `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Entry { …, #[serde(default, skip_serializing_if = "Option::is_none")] pub synced: Option<DateTime<Utc>> }
  impl Journal {
      pub fn pending(&self) -> io::Result<Vec<Entry>>;   // stage Applied && synced None, id order
      pub fn mark_synced(&self, id: &str, at: DateTime<Utc>) -> io::Result<()>;
      pub fn prune(&self, before: DateTime<Utc>) -> io::Result<Vec<String>>; // removes Applied entries with synced < before; returns ids
  }
  ```
  `FORMAT` stays 1: the field is optional, so existing entries read back unchanged. Prepared and
  Abandoned entries are never pruned. `Store::undo` keeps working on synced entries; it still
  refuses when a file no longer holds the entry's `after`.
- [ ] **Step 1: Write the failing tests:** `old_entry_without_synced_parses`; `pending_excludes_synced_prepared_and_abandoned`; `prune_removes_only_synced_before_cutoff`; `undo_works_on_synced_entry`. In `cli_op.rs`: `op_journal_shows_synced` (the line ends with `synced <RFC 3339>` or `pending`).
- [ ] **Step 2:** Run `cargo test -p twydo-core --test journal`. Expected: FAIL.
- [ ] **Step 3:** Implement. Replace the Retention section of `docs/journal.md`: an entry is pending until a sync confirms it (Task 7); it is pruned `retention_days` after confirmation.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(store): mark journal entries synced and prune them`.

### Task 3: Parse vdirsyncer output

**Files:**
- Create: `crates/twydo-core/src/sync/mod.rs`, `crates/twydo-core/src/sync/output.rs`, `crates/twydo-core/tests/data/vdirsyncer/*.txt`
- Test: `crates/twydo-core/tests/sync_output.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)] pub enum Action { Upload, Update, Delete }
  #[derive(Debug, Clone, PartialEq, Eq)] pub struct Transfer { pub action: Action, pub uid: String, pub storage: String, pub collection: String }
  #[derive(Debug, Default, Clone, PartialEq)] pub struct SyncOutput {
      pub synced: BTreeSet<String>,                    // "Syncing <pair>/<coll>"
      pub transfers: Vec<Transfer>,
      pub conflict_items: Vec<String>,                 // uids from "Doing conflict resolution for item <uid>..."
      pub unhooked: BTreeMap<String, Vec<String>>,     // coll → uids, from F4 ("One item changed on both sides" + "Item ID:")
      pub emptied: BTreeMap<String, String>,           // coll → emptied storage name (F7)
      pub errors: BTreeMap<String, Vec<String>>,       // coll → messages, continuations folded in (F3, other "error: <pair>/<coll>: …")
      pub unattributed: Vec<String>,                   // error:/critical: lines naming no collection (except the F2 "not completely equal" line)
  }
  pub fn parse(pair: &str, stderr: &str) -> SyncOutput;
  ```
  Rules: an `error:` line that does not start a new attributed error continues the previous
  attributed error, provided no non-error line came in between. Otherwise it is unattributed.
  `critical: The two files are not completely equal.` is dropped, because the hook log (Task 5)
  is the source of truth for unresolved conflicts. `warning:` lines are ignored. Collection
  names may contain spaces (`Syncing twydo/Twydo test`). Split `<storage>/<coll>` at the first
  `/`.
- [ ] **Step 1: Save fixtures.** Re-run the probe from "Facts" with storages named `twydo_remote` and `twydo_local`, pair `twydo`, collections `c1` and `Twydo test`, synthetic UIDs. Save each run's stderr with the temp paths replaced by `/tmp/vdirsyncer-conflict.XXXX`: `clean.txt`, `hook-unresolved.txt`, `hook-resolved.txt`, `same-content.txt`, `delete-vs-edit-local.txt`, `delete-vs-edit-remote.txt`, `hook-failed.txt`, `no-hook-conflict.txt`, `emptied-local.txt`, `force-delete.txt`, `two-collections.txt`.
- [ ] **Step 2: Write the failing tests**, one per fixture, asserting the exact fields. They include `emptied_names_storage` (`emptied["c1"] == "twydo_local"`), `no_hook_conflict_folds_continuations` (`unhooked["c1"] == ["n1"]`, and no unattributed lines), `hook_failed_is_collection_error`, `collection_with_space`, and `unknown_error_line_is_unattributed` (input `error: Connection refused`).
- [ ] **Step 3:** Run `cargo test -p twydo-core --test sync_output`. Expected: FAIL.
- [ ] **Step 4:** Implement `parse`.
- [ ] **Step 5:** Run it again. Expected: PASS.
- [ ] **Step 6:** Commit: `feat(sync): parse vdirsyncer output`.

### Task 4: Conflict store

**Files:**
- Create: `crates/twydo-core/src/conflicts.rs`
- Test: `crates/twydo-core/tests/conflicts.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)] #[serde(rename_all = "lowercase")]
  pub enum Side { Local, Remote }
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)] #[serde(rename_all = "kebab-case")]
  pub enum ConflictKind { BothChanged, LocalDeleteRemoteEdit, RemoteDeleteLocalEdit, Journal }
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)] #[serde(rename_all = "lowercase")]
  pub enum ConflictState { Open, Chosen, Resolved, Superseded }
  pub struct NewConflict { pub uid: String, pub collection: String, pub path: String /* vdir-relative */,
      pub kind: ConflictKind, pub held: bool, pub vdir_holds: Side,
      pub local: Option<String>, pub remote: Option<String> }        // None = deleted on that side
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
  pub struct Conflict { pub id: String, pub uid: String, pub collection: String, pub path: String,
      pub kind: ConflictKind, pub held: bool, pub vdir_holds: Side, pub detected: DateTime<Utc>,
      pub state: ConflictState, pub choice: Option<Side>, pub resolved: Option<DateTime<Utc>>,
      #[serde(skip)] pub local: Option<String>, #[serde(skip)] pub remote: Option<String> }
  pub struct ConflictStore { /* "<state>/conflicts/" */ }
  impl ConflictStore {
      pub fn open(state: &Path) -> io::Result<ConflictStore>;            // creates the dir
      pub fn existing(state: &Path) -> io::Result<Option<ConflictStore>>; // readers: creates nothing
      pub fn record(&self, c: NewConflict, now: DateTime<Utc>) -> io::Result<Conflict>;
      pub fn get(&self, id: &str) -> io::Result<Option<Conflict>>;
      pub fn list(&self) -> io::Result<Vec<Conflict>>;                     // id order, versions loaded
      pub fn unresolved(&self) -> io::Result<Vec<Conflict>>;               // Open or Chosen
      pub fn choose(&self, id: &str, side: Side) -> io::Result<()>;        // Open → Chosen
      pub fn mark_resolved(&self, id: &str, now: DateTime<Utc>) -> io::Result<()>;
      pub fn prune(&self, before: DateTime<Utc>) -> io::Result<Vec<String>>; // Resolved/Superseded with resolved < before
  }
  ```
  `held = true` means vdirsyncer still keeps the two sides apart (only `BothChanged`). Such a
  conflict is resolved by the hook (Task 5). Otherwise the vdir and the server already agree on
  the `vdir_holds` version, and resolving it is a normal journaled write (Task 9). `record`
  first returns an unresolved conflict for the same `path` with the same kind and bytes, if one
  exists (no duplicate per minute). It marks any other unresolved conflict for that `path`
  `Superseded`. Ids come from `journal::new_op_id`. Every write uses `write_atomic` under
  `conflicts/.lock`.
- [ ] **Step 1: Write the failing tests:** `record_writes_inspectable_files` (`conflict.json` parses, and `local.ics`/`remote.ics` hold the exact bytes); `deleted_side_has_no_file`; `record_is_idempotent_for_same_bytes`; `record_supersedes_older_for_path`; `existing_creates_nothing`; `prune_keeps_unresolved`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test conflicts`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): conflict store keeping both versions`.

### Task 5: The conflict hook

**Files:**
- Create: `crates/twydo-core/src/sync/hook.rs`, `crates/tdo/src/cli/conflicts.rs` (the `hook` subcommand only), `crates/tdo/tests/sync_e2e.rs`
- Modify: `crates/tdo/src/main.rs`, `crates/tdo/src/cli/mod.rs`

**Interfaces:**
- Consumes: `ConflictStore` (Task 4), `SyncConfig` (Task 1), `vdir::load`.
- Produces:
  ```rust
  pub const HOOK_LOG_ENV: &str = "TWYDO_SYNC_HOOKLOG";
  #[derive(Serialize, Deserialize, Debug, PartialEq)]
  pub struct HookRecord { pub uid: String, pub collection: String, pub conflict: String, pub resolved: bool }
  pub enum HookOutcome { Recorded(String /*conflict id*/), Applied(Side), Unchanged(String) }
  #[derive(thiserror::Error, Debug)] pub enum HookError { NotConfigured, UnknownSide(PathBuf, PathBuf), NoLocalFile(String), Io(io::Error) }
  pub fn conflict_hook(cfg: &Config, a: &Path, b: &Path, now: DateTime<Utc>) -> Result<HookOutcome, HookError>;
  pub fn read_hook_log(path: &Path) -> io::Result<Vec<HookRecord>>;
  ```
  CLI: `tdo conflicts hook <A> <B>` (hidden). Exit 0 after any `HookOutcome`; exit 2 on
  `HookError`, which per F3 leaves both sides untouched. vdirsyncer config line:
  `conflict_resolution = ["command", "tdo", "conflicts", "hook"]`.

  Algorithm: the local file is the argument whose file name equals `sync.local_storage`, and the
  other one is remote. If neither or both match → `UnknownSide`. Take the UID from the local
  version. Find the vdir file whose bytes equal the local version; if none, take the single file
  with that UID; otherwise `NoLocalFile`. Look for an unresolved held conflict for that path:
  - `Chosen` whose stored versions equal the two files → copy the chosen version over the other
    file → `Applied(side)`, logged `resolved: true`;
  - `Open` with equal versions → `Unchanged(id)`, logged `resolved: false`;
  - none, or different versions → `record` a held `BothChanged` (`vdir_holds: Local`) →
    `Recorded(id)`, logged `resolved: false`.

  Append the `HookRecord` as one JSON line to the file named by `$TWYDO_SYNC_HOOKLOG` when it is
  set. A user running vdirsyncer by hand has no log, and the conflict is still recorded.
- [ ] **Step 1: Write the failing tests.** In `sync_e2e.rs`, write a helper `Pair::new()`: a tempdir with `server/` (storage `twydo_remote`) and `vdir/` (storage `twydo_local`), collection `c1` pre-created on both sides (no discover prompt), a vdirsyncer config whose hook is `env!("CARGO_BIN_EXE_tdo") conflicts hook`, a tdo config with `[sync] vdirsyncer_config` pointing at it, and an initial sync of three synthetic tasks. Tests:
  - `hook_records_both_versions`: edit `u1` on both sides, run vdirsyncer → exit 1, one unresolved conflict, `local.ics` = vdir bytes, `remote.ics` = server bytes, other items synced.
  - `hook_does_not_duplicate_on_rerun`: run vdirsyncer twice → still one conflict.
  - `chosen_remote_is_applied_by_hook`: `ConflictStore::choose(id, Remote)` and run vdirsyncer → exit 0, the vdir file equals the server version.
  - `chosen_local_is_applied_by_hook`: same with `Local` → the server file equals the local version.
  - `remote_changed_again_supersedes`: choose, change the server copy again, run → the old conflict is `Superseded` and a new `Open` one exists.
  - `unknown_side_exits_2_and_changes_nothing`: call `tdo conflicts hook x y` directly.
- [ ] **Step 2:** Run `cargo test -p tdo --test sync_e2e`. Expected: FAIL.
- [ ] **Step 3:** Implement `conflict_hook` and the CLI.
- [ ] **Step 4:** Run it again. Expected: PASS (or `skipping` without vdirsyncer).
- [ ] **Step 5:** Commit: `feat(sync): conflict hook that keeps both versions`.

### Task 6: The runner, `tdo sync` and `tdo status`

**Files:**
- Create: `crates/twydo-core/src/sync/run.rs`, `crates/twydo-core/src/sync/status.rs`, `crates/tdo/src/cli/sync.rs`
- Modify: `crates/twydo-core/src/sync/mod.rs`, `crates/tdo/src/main.rs`
- Test: `crates/twydo-core/tests/sync_run.rs` (fake commands: small `sh` scripts that print given stderr, sleep or record `env`; `#[cfg(unix)]`), `crates/tdo/tests/cli.rs`

**Interfaces:**
- Consumes: Tasks 1–5; `recover`, `VdirLock`, `bind_vdir_state`.
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(tag = "state", rename_all = "kebab-case")]
  pub enum CollState { Ok, Conflict, EmptiedLocal, EmptiedRemote, Error { message: String } }
  #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
  pub struct SyncStatus { pub last_attempt: Option<DateTime<Utc>>, pub last_success: Option<DateTime<Utc>>,
      pub failures: u32, pub next_attempt: Option<DateTime<Utc>>, pub last_error: Option<String>,
      pub collections: BTreeMap<String, CollState>, pub metasync_error: Option<String> }
  impl SyncStatus { pub fn load(state: &Path) -> io::Result<SyncStatus>; pub fn save(&self, state: &Path) -> io::Result<()>; }
  pub fn backoff(failures: u32, max: Duration) -> Duration;   // 0 → 0; n → min(60 s · 2^(n−1), max)
  pub struct RunOptions { pub background: bool, pub force_delete: Vec<String> /* collection ids */ }
  pub enum Skip { NotConfigured, Busy, Backoff { until: DateTime<Utc> } }
  pub struct RunReport { pub skipped: Option<Skip>, pub collections: BTreeMap<String, CollState>,
      pub transient: Option<String>, pub new_conflicts: Vec<String>, pub confirmed: Vec<String>,
      pub pruned: usize, pub forced: Vec<String>, pub metasync_error: Option<String> }
  #[derive(thiserror::Error, Debug)] pub enum SyncError { Io(#[from] io::Error), Spawn(String) }
  pub fn run(cfg: &Config, opts: &RunOptions, now: DateTime<Utc>) -> Result<RunReport, SyncError>;
  pub struct SyncSummary { pub status: SyncStatus, pub pending: usize, pub conflicts: Vec<Conflict> }
  pub fn summary(cfg: &Config) -> io::Result<Option<SyncSummary>>;  // None without [sync]; creates nothing
  ```
  `run` order (each step names its lock):
  1. Run lock `<state>/sync/run.lock`. In background mode use `try_lock`. When busy, touch
     `sync/requested` and return `Skip::Busy`. Manual mode blocks.
  2. In background mode, if `status.next_attempt > now` → `Skip::Backoff`.
  3. `recover`. Turn each stuck entry into conflicts under the exclusive lock: every write whose
     file holds neither `before` nor `after` becomes `Journal`, `held: false`,
     `vdir_holds: Remote`, `local = write.after`, `remote = current`. Then mark the entry
     `Abandoned`. Writes already applied stay as they are (documented in `docs/sync.md`).
  4. Shared vdir lock. Read `journal.pending()` and the unresolved conflicts, truncate
     `sync/hook.log`, and spawn `<command> sync <pair>` with env `VDIRSYNCER_CONFIG` (when set)
     and `TWYDO_SYNC_HOOKLOG`, stdin null, stdout and stderr read by threads. Kill it after
     `timeout_secs`. Parse with Task 3. For forced collections run
     `<command> sync --force-delete <pair>/<coll>` instead of the pair. Handle emptied
     collections (Task 8). Then run `<command> metasync <pair>` if `metasync`; a failure only
     sets `metasync_error`. Release the shared lock.
  5. Exclusive vdir lock: `reconcile` (Task 7; until then a stub that confirms nothing).
  6. Set `CollState` per collection: `Error` if it has errors; `Conflict` if the hook log has
     an unresolved record for it or it is in `unhooked`; `Emptied*` from `emptied`; otherwise
     `Ok`. A run is a **transient failure** when the command could not spawn, timed out, or
     printed unattributed lines, or when any collection is `Error`. A transient failure
     increments `failures`, sets `next_attempt = now + backoff`, and sets `last_error` to its
     first line. Any other run sets `last_success = now` and `failures = 0`. Save
     `status.json`.
  7. If `sync/requested` exists, remove it and repeat from step 3, at most twice.

  CLI (exit 0 = all collections `Ok`; 1 = ran with conflicts, emptied lists or a failure; 2 =
  config or IO error, including "sync is not configured: add a [sync] table (docs/sync.md)"):
  ```
  tdo sync [--background] [--force-delete <LIST>]…   # LIST: collection id or display name
  tdo status                                          # last success, pending, conflicts, failures, per-list state
  ```
  `--background` prints nothing on success, for the timer's log.

  Also in `tdo::cli::sync`: `pub fn spawn_background(config: &Config, vdir_flag: Option<&Path>)`.
  When `config.sync` is set, it starts `current_exe() [--vdir <flag>] sync --background`
  detached (stdin, stdout and stderr null; never waited on). Errors are ignored: the timer
  catches up. Tasks 9, 10 and 11 call it.
- [ ] **Step 1: Write the failing tests** in `sync_run.rs`:
  - `not_configured_skips`;
  - `clean_run_sets_last_success` (fake prints `clean.txt`, exit 0);
  - `vdirsyncer_config_is_passed_as_env` (fake writes `$VDIRSYNCER_CONFIG` to a file);
  - `failure_backs_off_and_background_respects_it` (fake exit 1 with `error: Connection refused`; `failures == 1`, `next_attempt = now + 60 s`; a background run at `now + 30 s` → `Skip::Backoff`; a manual run is not skipped);
  - `backoff_caps_at_max` (unit test of `backoff`);
  - `hung_command_is_killed_after_timeout` (`timeout_secs = 1`, fake `sleep 30`; returns within 5 s, transient set, the shared lock is released afterwards: `VdirLock::exclusive` succeeds);
  - `busy_background_run_requests_rerun` (hold the run lock in the test; a background run → `Skip::Busy` and `sync/requested` exists; a manual run in another thread then runs twice, counted by the fake);
  - `stuck_journal_entry_becomes_conflict` (a Prepared entry whose file holds a third content → a `Journal` conflict, the entry `Abandoned`, the vdir untouched);
  - `writers_wait_for_sync` (while the fake sleeps 1 s, `VdirLock::exclusive` in another thread blocks until it ends; `shared` does not).

  In `cli.rs`: `sync_without_config_exits_2`; `status_prints_never_synced`;
  `spawn_background_starts_runner` (`[sync] command` = a fake that touches a marker; the marker appears within 5 s).
- [ ] **Step 2:** Run `cargo test -p twydo-core --test sync_run`. Expected: FAIL.
- [ ] **Step 3:** Implement `run`, `SyncStatus`, `summary` and the CLI.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(sync): serialized sync runner with backoff and status`.

### Task 7: Confirm, detect delete-versus-edit, prune

**Files:**
- Create: `crates/twydo-core/src/sync/reconcile.rs`
- Modify: `crates/twydo-core/src/sync/run.rs` (replace the stub)
- Test: `crates/tdo/tests/sync_e2e.rs`, `crates/twydo-core/tests/sync_run.rs`

**Interfaces:**
- Consumes: `SyncOutput`, `HookRecord`, `Journal::{pending, mark_synced, prune}`, `ConflictStore`.
- Produces:
  ```rust
  pub struct Reconciled { pub confirmed: Vec<String>, pub new_conflicts: Vec<String>, pub resolved: Vec<String> }
  pub fn reconcile(vdir: &Path, journal: &Journal, conflicts: &ConflictStore, sync: &SyncConfig,
      pending: &[Entry], out: &SyncOutput, hook: &[HookRecord], now: DateTime<Utc>) -> io::Result<Reconciled>;
  ```
  Runs under the exclusive lock. Only collections that are `Ok` or `Conflict` in this run count,
  and nothing counts when `out.unattributed` is non-empty. For each path written by the
  `pending` entries in a counted collection, in order:
  1. **Skip** the path if a journal entry *not* in `pending` now touches it (a write that slipped
     in between the two locks). It is checked next run.
  2. The **expected** content is the `after` of the last pending entry writing the path. The
     path was **created here** if any pending entry writes it with `before: None`. Its **uid**
     comes from the expected content, or from the last `before` when the expected content is a
     deletion.
  3. An unresolved held conflict for the path whose collection is still `Conflict` → leave the
     path pending.
  4. A `Transfer { Upload, storage ≠ local_storage }` for the uid, when the path was not created
     here → **remote delete versus local edit** (F6). Record `RemoteDeleteLocalEdit`,
     `held: false`, `vdir_holds: Local`, `local = expected`, `remote = None`.
  5. The current file differs from the expected content (absent counts as `None`) → if the
     expected content is `None`, **local delete versus remote edit** (F6): record
     `LocalDeleteRemoteEdit`, `vdir_holds: Remote`, `local = None`, `remote = current`.
     Otherwise leave it pending. A non-deletion that differs with no conflict is unexpected;
     log it and do not confirm it.
  6. Otherwise the path is confirmed.

  Mark an entry synced when every path it writes was confirmed or turned into a conflict in
  this step. Then: held conflicts with a `resolved: true` hook record, or with no hook record
  this run while their collection is `Ok` (both sides now agree, F5), are `mark_resolved`, and
  pending entries writing their path are marked synced (the user decided). Finally `prune`
  journal entries and conflicts older than `retention_days`.
- [ ] **Step 1: Write the failing tests** in `sync_e2e.rs`. Twydo writes go through `tdo op` (test mode, test matrix, as in `cli_op.rs`); "phone" writes edit `server/` directly:
  - `edit_is_confirmed_after_sync` (`tdo op edit`, `tdo sync` → `tdo op journal` shows `synced`, the server holds the edit);
  - `local_delete_vs_remote_edit_becomes_conflict` (`tdo op delete u2`, edit `u2` on the server, `tdo sync` → exit 1, a `local-delete-remote-edit` conflict with `remote.ics` = the edit and no `local.ics`; the entry is synced);
  - `remote_delete_vs_local_edit_becomes_conflict` (mirror: `remote-delete-local-edit`, `local.ics` = the edit);
  - `created_task_upload_is_not_a_conflict` (`tdo op add` then sync → no conflict);
  - `held_conflict_keeps_entry_pending_until_resolved` (Twydo edit vs server edit → the entry stays pending; choose `Remote`, sync → the entry is synced and the conflict `resolved`);
  - `agreeing_sides_resolve_open_conflict` (edit the server copy back to the local bytes → resolved without a choice);
  - `prune_after_retention` (`retention_days = 0` → the confirmed entry is gone on the next run).

  In `sync_run.rs`: `unattributed_error_confirms_nothing`; `errored_collection_stays_pending_then_confirms` (spec §9 partial upload: a run where `c1` is `Error` confirms nothing in `c1`; the next clean run confirms it, and no second history copy appears); `write_between_locks_stays_pending` (call `reconcile` with a `pending` list that lacks a newer entry for the path).
- [ ] **Step 2:** Run `cargo test -p tdo --test sync_e2e` and `cargo test -p twydo-core --test sync_run`. Expected: FAIL.
- [ ] **Step 3:** Implement `reconcile` and call it from `run`.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(sync): confirm journal entries and catch delete-versus-edit`.

### Task 8: Emptied collections

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs`
- Test: `crates/tdo/tests/sync_e2e.rs`

**Interfaces:**
- Consumes: `SyncOutput.emptied`, `pending`, unresolved conflicts.
- Produces the rule. `emptied[coll] == local_storage` is **local emptied**. Force it
  automatically, by rerunning `<command> sync --force-delete <pair>/<coll>` once in the same
  run, only when a pending entry writes `after: None` into `coll` (Twydo removed the last task)
  and `coll` has no unresolved conflict. Report it in `RunReport.forced`. Any other emptied
  collection is `EmptiedLocal` or `EmptiedRemote` and is never forced. `tdo sync` prints
  `list "<name>" was emptied on the server; to apply that here run: tdo sync --force-delete "<name>"`
  (local: `…emptied here…`). An explicit `--force-delete` is the user's confirmation.
- [ ] **Step 1: Write the failing tests:**
  - `twydo_emptying_a_list_is_forced` (`tdo op delete` every task in `c1` → one `tdo sync`, exit 0, the server collection empty, `forced == ["c1"]`);
  - `remote_emptied_is_never_forced` (delete every server file → exit 1, local tasks still there, status `emptied-remote`, the hint printed; then `tdo sync --force-delete c1` → local emptied, exit 0);
  - `local_emptied_without_journal_is_not_forced` (remove the vdir files by hand → `emptied-local`, the server untouched).
- [ ] **Step 2:** Run `cargo test -p tdo --test sync_e2e`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(sync): handle emptied collections without losing tasks`.

### Task 9: Resolving conflicts from the CLI

**Files:**
- Create: `crates/twydo-core/src/sync/resolve.rs`
- Modify: `crates/twydo-core/src/store/mod.rs`, `crates/tdo/src/cli/conflicts.rs`
- Test: `crates/tdo/tests/sync_e2e.rs`, `crates/twydo-core/tests/store.rs`

**Interfaces:**
- Produces:
  ```rust
  // store/mod.rs: writes one file through the journal (kind "resolve"), without the gate:
  // it restores a version that 2Do or Twydo already wrote. Stale if the file no longer holds `expect`.
  pub fn restore(&mut self, path: &str, expect: Option<&str>, content: Option<&str>,
      uids: Vec<String>, now: DateTime<Utc>) -> Result<String, WriteError>;
  // sync/resolve.rs
  pub enum Resolution { Recorded, AlreadyInPlace, Written(String /* op id */) }
  #[derive(thiserror::Error, Debug)] pub enum ResolveError { NotFound(String), NotUnresolved(String), Write(#[from] WriteError), Io(#[from] io::Error) }
  pub fn resolve(cfg: &Config, id: &str, keep: Side, now: DateTime<Utc>) -> Result<Resolution, ResolveError>;
  ```
  Held → `choose` → `Recorded`; the hook applies it on the next sync. Not held and
  `keep == vdir_holds` → `mark_resolved` → `AlreadyInPlace`. Otherwise `Store::restore(path,
  expect = vdir_holds version, content = kept version)`, then `mark_resolved` → `Written`; the
  next sync propagates it as an ordinary change.

  CLI:
  ```
  tdo conflicts                                   # id, list, title (from either version), kind in words, state
  tdo conflicts show <ID>                         # both versions' SUMMARY, DUE, STATUS, LAST-MODIFIED, then the file paths
  tdo conflicts resolve <ID> --keep local|remote  # then starts a background sync (Task 10's spawn_background)
  ```
  Kind in words: `changed here and on another device` · `deleted here, edited on another device` ·
  `edited here, deleted on another device` · `an interrupted change here meets another device's version`.
- [ ] **Step 1: Write the failing tests:** `restore_writes_through_journal_and_refuses_stale` (in `store.rs`); in `sync_e2e.rs`:
  - `keep_deletion_deletes_everywhere` (from `local_delete_vs_remote_edit`: `--keep local`, sync → gone from the vdir and the server);
  - `keep_edit_after_remote_delete_is_noop` (`--keep local` on `remote-delete-local-edit` → `AlreadyInPlace`);
  - `keep_remote_deletion_removes_local` (`--keep remote` on the same → the file is removed and the server has none);
  - `resolve_held_via_cli` (`--keep remote` on `both-changed`, sync → the vdir holds the server version);
  - `conflicts_lists_kind_in_words`.
- [ ] **Step 2:** Run `cargo test --workspace`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(sync): resolve conflicts by keeping either version`.

### Task 10: Sync after edits

**Files:**
- Modify: `crates/tdo/src/cli/op.rs`
- Test: `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Consumes: `spawn_background` (Task 6).
- Produces the rule: when `config.sync` has `after_edit`, `tdo op` calls `spawn_background`
  after every exit 0 that wrote (not after `check`, `journal`, `recover` or `abandon`). Plan 5
  applies the same rule to TUI writes.
- [ ] **Step 1: Write the failing tests:** `op_triggers_background_sync` (`[sync] command` = a fake that touches a marker; after `tdo op edit` the marker appears within 5 s); `op_check_does_not_trigger_sync`; `after_edit_false_does_not_trigger`.
- [ ] **Step 2:** Run `cargo test -p tdo --test cli_op`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tdo): sync in the background after each write`.

### Task 11: TUI: sync status, `:sync`, conflict warnings and `:conflicts`

**Files:**
- Create: `crates/tdo/src/ui/conflicts.rs`
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/term.rs`, `crates/tdo/src/ui/{mod,status,tasks,details}.rs`
- Test: `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `summary`, `resolve`, `spawn_background`.
- Produces:
  ```rust
  pub enum Outcome { Continue, Quit, Sync { force_delete: Option<String> }, Resolve { id: String, keep: Side } }
  impl App { pub fn set_sync(&mut self, s: Option<SyncSummary>); pub fn sync_finished(&mut self, message: String); }
  pub struct App { …, pub sync: Option<SyncSummary>, pub show_conflicts: bool, pub conflict_cursor: usize }
  ```
  The term loop runs `Outcome::Sync` as a child `tdo sync [--force-delete X]` on a thread. When
  it ends it calls `sync_finished`, with the child's first stderr line or `sync done`.
  `Outcome::Resolve` calls `resolve` and then `spawn_background`. `summary` is reloaded on
  every tick, every reload and every `sync_finished`. At startup the loop calls
  `spawn_background` once.

  Status segment, after the unreadable indicator, ANSI colours only:
  - `synced 3m ago` (`just now` under 1 min, then `Nm`, `Nh`, `Nd`) or `not synced yet`;
  - ` · N pending` when N > 0;
  - ` · N conflict(s)` in yellow;
  - ` · "<list>" emptied on server (:sync! <list>)` in yellow;
  - on a transient failure the segment is `sync failed: <last_error>; retry HH:MM` in red,
    with local time from `next_attempt`.

  Commands: `:sync` runs a sync. `:sync! <list>` runs one with `--force-delete`. `:conflicts`
  toggles the overlay. The overlay lists unresolved conflicts: the list name, the title, the
  kind in words (Task 9), and a `waiting for sync` tag on `Chosen`. Keys: `j`/`k`, `l` keep
  local, `r` keep remote, `Esc` close. A task row whose UID has an unresolved conflict starts
  with a yellow `!`. The details pane adds a line `! conflict: <kind in words> — :conflicts`.
- [ ] **Step 1: Write the failing tests:**
  - `app_keys.rs`: `sync_command_returns_sync_outcome`; `sync_bang_passes_list`; `conflicts_overlay_keys_return_resolve` (`l` → `Resolve { keep: Local }`); `esc_closes_conflicts`.
  - `ui_render.rs` (TestBackend, as existing): `status_shows_synced_ago_pending_and_conflicts`; `status_shows_failure_with_retry_time`; `conflicted_row_has_marker`; `details_show_conflict_line`; `conflicts_overlay_lists_kind_in_words`.
  - Update the existing tests that expected `sync: not available yet`.
- [ ] **Step 2:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(tui): sync status, :sync and conflict resolution`.

### Task 12: Background setup and docs

**Files:**
- Create: `contrib/systemd/twydo-sync.service`, `contrib/systemd/twydo-sync.timer`, `contrib/launchd/app.twydo.sync.plist`, `docs/sync.md`
- Modify: `contrib/vdirsyncer/config.example`

- [ ] **Step 1:** Write the units.
  - Service: `Type=oneshot`, `ExecStart=%h/.cargo/bin/tdo sync --background`, with a comment to adjust the path. For the test setup, a commented `Environment=XDG_CONFIG_HOME=%h/twydo-phase0/xdg`.
  - Timer: `OnBootSec=1min`, `OnUnitActiveSec=1min`, `AccuracySec=5s`, `WantedBy=timers.target`.
  - Plist: `Label app.twydo.sync`, `ProgramArguments [/usr/local/bin/tdo, sync, --background]` (placeholder path), `StartInterval 60`, `RunAtLoad true`, `StandardErrorPath /Users/USER/Library/Logs/twydo-sync.log` (placeholder; launchd does not expand `~`).
- [ ] **Step 2:** In `config.example`, replace the "Deliberately NO conflict_resolution" comment with `conflict_resolution = ["command", "tdo", "conflicts", "hook"]`. Explain that it is not `a wins`/`b wins`: the hook keeps both versions and changes nothing until the user chooses (docs/sync.md).
- [ ] **Step 3:** Write `docs/sync.md`:
  - setup: the vdirsyncer hook line, `[sync]`, enabling the timer or agent;
  - what `tdo sync` does, per the `run` order in Task 6;
  - each conflict kind and what each choice does;
  - emptied lists;
  - backoff and timeout;
  - the state files;
  - the known limits: a `Journal` conflict leaves already-applied writes of that entry in place; the no-etag delete warning; vdirsyncer run by hand outside Twydo records conflicts but confirms nothing.

  Link it from `docs/config.md`.
- [ ] **Step 4:** Verify: `systemd-analyze --user verify contrib/systemd/twydo-sync.timer contrib/systemd/twydo-sync.service` (expected: no errors apart from the placeholder binary path), and `python3 -c "import plistlib,sys; plistlib.load(open(sys.argv[1],'rb'))" contrib/launchd/app.twydo.sync.plist` (expected: exit 0).
- [ ] **Step 5:** Commit: `docs(sync): background units, vdirsyncer hook and sync guide`.

### Task 13: Verify against Synology and the iPhone (👤 user + 🤖, test account only)

**Files:**
- Create: `docs/reference/phase0/plan3-sync.md`; anonymised CalDAV stderr fixtures `crates/twydo-core/tests/data/vdirsyncer/caldav-*.txt`
- Modify: `crates/twydo-core/tests/sync_output.rs` (one test per new fixture), `AGENTS.md` ("What to do next"), the memory note

**Stop and confirm with the user** before starting. This task changes `~/.config/vdirsyncer/config`
(adds the hook line), installs the timer with `Environment=XDG_CONFIG_HOME=%h/twydo-phase0/xdg`,
and uses the test lists `Twydo test` / `Twydo test 2` with single-letter synthetic titles. It
runs in rounds of at most 6 actions, as the user prefers.

- [ ] **Step 1:** Write the checklist in `plan3-sync.md`. Each round lists the actions, the expected Twydo result (`tdo status`, `tdo conflicts`) and the expected iPhone result:
  1. edit vs edit (Twydo `tdo op edit` + phone edit before sync) → a `both-changed` conflict; resolve `remote` once and `local` once; the phone shows the chosen version;
  2. concurrent completion of a monthly task (Twydo + phone before sync) → record what happens to the live task and the two history copies (spec §9; no fix is planned here, only an observation);
  3. Twydo delete vs phone edit → `local-delete-remote-edit`; `--keep local` → gone on the phone;
  4. phone delete vs Twydo edit → `remote-delete-local-edit`; `--keep remote` → gone in the vdir;
  5. the phone deletes the last task of `Twydo test 2` → `emptied-remote`, nothing forced; `tdo sync --force-delete` → the vdir is empty; Twydo moves the last task out → forced automatically;
  6. offline (wrong host in a copy of the vdirsyncer config pointed to by `vdirsyncer_config`) → backoff and the failure in `tdo status` and the TUI; restore → recovery; capture the error output;
  7. the timer with the TUI closed: a phone edit reaches the vdir within about 2 minutes; renaming the list on the phone reaches `displayname` through metasync;
  8. watch for the iPhone's "invalid task reference" error after deletes and record whether it appears.
- [ ] **Step 2 (👤+🤖):** Run it. Save each round's vdirsyncer stderr. Anonymise it (host, user, collection hrefs) into `caldav-*.txt`, check it with `python3 tools/capture_fixture.py scan`-style review, and have the user confirm it.
- [ ] **Step 3:** Add parser tests for the new fixtures. Fix any line format the CalDAV run shows differently from F1–F8. Record findings and anything surprising in `plan3-sync.md`. Open a `gh issue` for any fix that needs code beyond this plan.
- [ ] **Step 4:** Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`. Expected: PASS.
- [ ] **Step 5:** Commit: `docs(phase0): verify sync and conflicts against iPhone 2Do`. Update AGENTS.md "What to do next" and the memory note to point at Plan 5 (or Plan 2).
