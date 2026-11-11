# List Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Twydo adopts lists created on the phone: discovery at the daemon's startup sync and on `tdo sync --discover` / `:discover`, creating local folders only.

**Architecture:** A new `discover` action in the bundled Python helper calls vdirsyncer's `collections_for_pair(from_cache=False)` with a missing-collection policy that creates only on the local filesystem storage. A Rust wrapper (`twydo_core::sync::discover`) invokes it; `sync::run` calls it before `vdirsyncer sync` when `RunOptions::discover` is set or the `sync/rediscover` gate is pending. The daemon sets the flag for the startup sync and for `sync.run` with `discover: true`.

**Tech Stack:** Rust (twydo-core, twydo-daemon, tdo), Python 3 helper against vdirsyncer 0.21, unittest.

**Spec:** `docs/superpowers/specs/2026-11-11-list-discovery-design.md`

## Global Constraints

- Only the local (filesystem) side may be created. A collection missing on the remote side is refused, never created.
- Never answer vdirsyncer prompts wholesale; never pipe `yes`.
- Ordinary periodic, batched and plain manual syncs do not discover.
- The `sync/rediscover` gate keeps running plain `vdirsyncer discover` with today's semantics; the helper discovery runs before it, and a helper failure there is a warning.
- `tdo sync --discover` failure → `SyncOutcome::Failed`, exit 1. Startup discovery failure → warning only.
- Helper replies never contain URLs or raw exception text; only protocol codes and plain local collection ids (`plain_id`).
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; no cargo target under `/tmp`.
- `python3 tools/check_links.py` must pass after doc edits.
- Commits: conventional style, no agent attribution lines.

## Review Focus

1. **Server collection id that is not a `plain_id`** (contains `/`, `%`, starts with `.`): must be refused with code `mapping`, never mkdir'd. Test in Task 1.
2. **Remote-missing collection during startup discovery**: daemon keeps running, the startup sync still syncs the cached lists, and the outcome is not `Failed`. Test in Task 3.
3. **A pair the helper refuses (non-CalDAV remote, as in `sync_e2e.rs`)**: the `:forget-list` gate still behaves exactly as today. Existing `sync_e2e.rs` rediscovery tests must pass unchanged (Task 2).
4. **Discovery run on a rerun pass**: discovery happens once per `run`, in pass 0 only. Test in Task 2.
5. **New list reaches clients**: after an adopting sync the daemon's snapshot and the TUI list pane include the new collection with its metasynced name. Test in Task 3.

---

### Task 1: Helper `discover` action and Rust wrapper

**Files:**
- Modify: `crates/twydo-core/src/lists/helper.py` (`validate_request`, `pair_context`, `refresh_discovery`, `dispatch`, `main`, `Refused`)
- Modify: `crates/twydo-core/src/lists/helper.rs` (reply field, `HelperRequest::discover`, split transport from code mapping)
- Create: `crates/twydo-core/src/sync/discover.rs`; register in `crates/twydo-core/src/sync/mod.rs`
- Modify: `crates/twydo-core/tests/common/list_backend.py` (scripted discovery)
- Test: `tools/tests/test_list_helper.py`, `crates/twydo-core/tests/sync_discover.rs` (new)

**Interfaces:**
- Produces (Python): request `{"action": "discover", "mode": "execute", ...probe fields}`, no `collection`, `operation` or `state`. Reply ok: `{"result": {"refreshed": true, "adopted": [<ids>]}}`. Reply refused: `{"ok": false, "code": "discovery", "missing": [<id>]}` for a remote-missing collection; other codes unchanged.
- Produces (Rust):
  - `HelperReply { ..., #[serde(default)] missing: Vec<String> }`
  - `HelperRequest::discover(cfg: &Config) -> Result<HelperRequest, ListError>` (probe with `action = "discover"`)
  - `pub(crate) fn exchange(cfg: &Config, req: &HelperRequest) -> Result<HelperReply, ListError>`: transport only, returns non-ok replies; `invoke` = `exchange` + existing code→message mapping (moved into `fn refusal(code: &str) -> &'static str`).
  - `twydo_core::sync::discover::{discover, Discovered, DiscoverError}`:
    ```rust
    pub struct Discovered { pub adopted: Vec<String> }
    pub enum DiscoverError { Missing { id: String }, Failed(String) }
    pub fn discover(cfg: &Config) -> Result<Discovered, DiscoverError>
    ```
    `Missing` when the code is `discovery` and `missing` has one plain id; `Failed(refusal(code))` otherwise; transport errors become `Failed(e.to_string())`.

- [ ] **Step 1: Python tests (fail)** in `tools/tests/test_list_helper.py`:
  - `test_discover_request_needs_no_collection_or_operation`: `validate_request(dict(version=1, action="discover", pair="test", local_storage="local", vdir="/tmp/x"))` returns; the same with `mode="refresh"` raises `Refused("protocol")`.
  - `test_discover_policy_creates_only_local_filesystem`: build the policy via the new `local_only_policy(local_storage, original, adopted)` and await it:
    - `({"type": "filesystem", "instance_name": "local"}, "phone-list")` calls `original(..., implicit_create=True)` once, appends `"phone-list"` to `adopted`
    - `({"type": "caldav", "instance_name": "remote"}, "stale")` raises `Refused` with `.code == "discovery"` and `.missing == ["stale"]`, `original` not called
    - `({"type": "filesystem", "instance_name": "local"}, "../x")` raises `Refused("mapping")`, `original` not called
- [ ] **Step 2: Run** `python3 -m unittest tools.tests.test_list_helper -v`. Expected: the three new tests FAIL (`AttributeError`/`Refused("protocol")`).
- [ ] **Step 3: Implement in `helper.py`.**
  - `Refused.__init__(self, code="mapping", missing=None)`.
  - Accept `"discover"` in `validate_request`, with `mode == "execute"` only and no `collection` requirement.
  - `pair_context`: skip the per-collection path checks and `remote_identity` for `discover`. Return `identity=None`. Keep the pair, storage-type, `url.fetch`/`path.fetch` and status-path checks.
  - `local_only_policy(local_storage, original, adopted)` returns the async handler `(config, collection, e=None, implicit_create=False)`.
  - `refresh_discovery` gains `policy=None`. The default stays `refuse_missing`.
  - In `dispatch`, `action == "discover"` runs `refresh_discovery` with the policy and returns `{"refreshed": True, "adopted": adopted}`.
  - `main` adds `"missing": error.missing` to the refused reply when it is set.
  - `operation_guard` already yields `None` without `operation`. The helper takes no locks for `discover`, because the sync run holds them.
- [ ] **Step 4: Run** the unittest command. Expected: all PASS.
- [ ] **Step 5: Rust test (fail)** `crates/twydo-core/tests/sync_discover.rs`, reusing the `offline()` harness from `lists_remote.rs`. Move `offline()` and `flag()` into `tests/common/lists.rs` and use them from both files. Extend `list_backend.py`'s `collections_for_pair` to read `store["discover"] = {"remote_only": [...], "local_only": [...]}`:
  - For each `remote_only` name missing in the vdir, it awaits `discovery.handle_collection_not_found(local_storage_cfg, name)`.
  - For each `local_only` name, it awaits the same with the remote config.
  - The fake `utils.handle_collection_not_found` honours `implicit_create=True` for filesystem by `mkdir`.

  Tests:
  - `discover_adopts_a_remote_only_list`: `remote_only=["phone-list"]` → `Ok(Discovered{adopted: ["phone-list"]})`, `cfg.vdir.join("phone-list").is_dir()`.
  - `discover_never_creates_a_remote_list`: `local_only=["stale"]` → `Err(DiscoverError::Missing{id: "stale"})`; `server.json` `calls` contains no `MKCALENDAR`.
  - `discover_maps_other_refusals`: `flag("fail_refresh_once")` → `Err(DiscoverError::Failed(_))` and the message has no `https://`.
- [ ] **Step 6: Run** `cargo test -p twydo-core --test sync_discover --test lists_remote`. Expected: new tests FAIL to compile, then implement `helper.rs` + `sync/discover.rs` per Interfaces.
- [ ] **Step 7: Run** `cargo test -p twydo-core --test sync_discover --test lists_remote --test lists_helper` and the Python suite. Expected: PASS.
- [ ] **Step 8: Commit** `feat(lists): add local-only collection discovery to the helper`.

### Task 2: Discovery inside the sync run

**Files:**
- Modify: `crates/twydo-core/src/sync/run.rs` (`RunOptions`, `RunReport`, `run`, `one_pass`)
- Modify: `crates/twydo-core/src/sync/report.rs` (`messages`, `list_names`)
- Test: `crates/twydo-core/tests/sync_run.rs`

**Interfaces:**
- Consumes: `sync::discover::discover` (Task 1).
- Produces:
  - `RunOptions { ..., pub discover: bool }`: discover before the first pass's `vdirsyncer sync`.
  - `RunReport { ..., pub adopted: Vec<String>, pub discovery_error: Option<String> }`
  - `messages()`:
    - A note `found new list "<display name>"` for each adopted id.
    - `discovery_error` goes to **problems** when `!opts.background` and to **warnings** otherwise. Record which one in `RunReport` as `discovery_is_warning: bool`, set by `run`.

- [ ] **Step 1: Tests (fail)** in `sync_run.rs`. Use `Fx` with the fake `vdirsyncer` script, plus `list_python` from the offline wrapper:
  - `discover_option_adopts_before_sync`: `remote_only=["phone-list"]`, `RunOptions{discover: true, ..manual()}`.
    - `report.adopted == ["phone-list"]`
    - the `calls` file shows `sync` ran after discovery
    - `messages(...).notes` contains `found new list`
  - `discover_failure_on_manual_run_is_a_problem_and_sync_still_runs`: `local_only=["stale"]`.
    - `calls` contains `sync`
    - `messages(...).problems` has one entry containing `stale` and `:forget-list stale`
  - `discover_failure_on_background_run_is_a_warning`: the same with `background: true`. `problems` is empty and `warnings` names `stale`.
  - `discovery_runs_once_across_rerun_passes`: create `sync/requested` during the first `sync` call (script touches it). `server.json` shows exactly one discovery: count a `"discover"` entry the fake appends to `calls`.
  - `ordinary_run_does_not_discover`: `manual()`, then no discovery recorded.
  - `rediscover_gate_runs_helper_then_plain_discover`: marker present, `remote_only=["phone-list"]`. The helper adopts, then the script's `discover` call follows, then `sync`.
  - The existing `failed_rediscovery_*` tests and `discovery_marks_only_active_lists_missing_from_remote_storage` stay unchanged and green. Their configs have no helper backend; the helper failure becomes a warning.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test sync_run discover`. Expected: FAIL (no field `discover`).
- [ ] **Step 3: Implement.**
  - In `one_pass`, take a `discover: bool` set for pass 0 when `opts.discover || rediscover.exists()`.
  - Under the shared vdir lock, before the gate's plain discover, call `discover(cfg)`. Fill `report.adopted`.
  - For `DiscoverError::Missing{id}`, set the message `list discovery stopped: list "<name or id>" ({id}) no longer exists on the server; if you deleted it, run :forget-list {id}`.
  - For `Failed(m)`, set `list discovery failed: {m}`.
  - When only the gate triggered discovery, any failure is a warning.
  - `list_names` also loads names when `adopted` is non-empty.
- [ ] **Step 4: Run** `cargo test -p twydo-core --test sync_run && cargo test -p tdo --test sync_e2e`. Expected: PASS (e2e unchanged).
- [ ] **Step 5: Commit** `feat(sync): discover new server lists before syncing on request`.

### Task 3: Daemon: startup discovery and `sync.run { discover }`

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/sync.rs` (`SyncRunParams`)
- Modify: `crates/twydo-daemon/src/server/syncjob.rs` (`sync_run`)
- Modify: `crates/twydo-daemon/src/server/jobs.rs` (`run_background`)
- Modify: `schema/tdo.openrpc.json` (regenerated)
- Test: `crates/twydo-daemon/tests/sync.rs`

**Interfaces:**
- Consumes: `RunOptions::discover` (Task 2).
- Produces: `SyncRunParams { request_id, force_delete, #[serde(default)] discover: bool }`, used by Task 4's clients.

- [ ] **Step 1: Tests (fail)** in `daemon/tests/sync.rs`, using the existing daemon test harness with the offline helper backend and fake vdirsyncer:
  - `startup_sync_discovers_new_lists`: post `SyncReason::Startup` via `post_sync_for_test`, with `remote_only=["phone-list"]`. After the sync ends, the snapshot's lists include `phone-list`, and the `SyncResult` notes contain `found new list`.
  - `startup_discovery_failure_is_a_warning`: `local_only=["stale"]`. The outcome is `Ok`, `warnings` names `stale`, and the daemon still answers `rpc.discover`.
  - `periodic_sync_does_not_discover`: `SyncReason::Periodic` makes no discovery call.
  - `sync_run_with_discover_fails_on_missing_remote`: `sync.run {discover: true}` with `local_only=["stale"]` gives `SyncOutcome::Failed`, with a problem containing `:forget-list stale`.
- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test sync discover`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `run_background` sets `discover: reason == SyncReason::Startup`.
  - `sync_run` passes `p.discover`.
  - `lists.rs:310` keeps the default.
- [ ] **Step 4: Regenerate schema:** `TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema`, then `cargo test -p twydo-daemon`. Expected: PASS, including the released-schema compatibility check (additive optional field).
- [ ] **Step 5: Commit** `feat(daemon): discover lists at startup and on request`.

### Task 4: CLI flag, TUI command, docs

**Files:**
- Modify: `crates/tdo/src/main.rs` (`Command::Sync`)
- Modify: `crates/tdo/src/cli/sync.rs` (`sync`, `run_and_wait`)
- Modify: `crates/tdo/src/app.rs` (`Outcome::Sync`, command match)
- Modify: `crates/tdo/src/term.rs`, `crates/tdo/src/term/link.rs` (`Command::Sync`)
- Modify: `crates/tdo/src/ui/mod.rs` (help list near `":forget-list [list]"`)
- Modify: `docs/sync.md`, `docs/tui.md`, `docs/compatibility.md` (collection management paragraph)
- Test: `crates/tdo/tests/cli.rs`, `crates/tdo/tests/app_keys.rs` (or wherever `:sync!` parsing is tested)

**Interfaces:**
- Consumes: `SyncRunParams::discover` (Task 3).
- Produces:
  - `--discover` on `tdo sync`, conflicting with `switch` and `background`.
  - `Outcome::Sync { force_delete: Option<String>, discover: bool }`
  - `link::Command::Sync { force_delete: Vec<String>, discover: bool }`

- [ ] **Step 1: Tests (fail):**
  - The app command test asserts `:discover` returns `Outcome::Sync { force_delete: None, discover: true }`.
  - `:discover x` sets the message `usage: :discover`.
  - `:sync` still gives `discover: false`.
  - The CLI test asserts `tdo sync --discover on` is rejected by clap, and that `tdo sync --discover` sends `discover: true`. Use the existing CLI/daemon test pattern for `--force-delete`.
- [ ] **Step 2: Run** `cargo test -p tdo discover`. Expected: FAIL.
- [ ] **Step 3: Implement** the wiring. Add the help line `":discover", "find lists created on another device, then sync"`.
- [ ] **Step 4: Docs.**
  - `docs/sync.md`: add a section "New lists from other devices". Cover:
    - discovery runs at daemon start (skipped when automatic sync is off or backing off), and via `tdo sync --discover` / `:discover`
    - only local folders are created
    - a stale list stops discovery, with the `:forget-list` hint
    - the gate now runs the local-only discovery before plain discovery
    - setup no longer needs a manual `discover` after the first one
  - `docs/tui.md`: add `:discover` to the command table.
  - `docs/compatibility.md`: Twydo now picks up phone-created lists. Verified-mode creation stays refused until the device round.
  - Run `python3 tools/check_links.py`.
- [ ] **Step 5: Run** `cargo test --workspace -- --test-threads=4` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 6: Commit** `feat(tdo): add tdo sync --discover and :discover`.

### Task 5: Device-round checklist (stop for approval)

No code. Requires Tasks 1–4 merged-ready and a release build installed for the test setup.

- [ ] **Step 1:** Check `compat/matrix.toml` (`list-create.collection`) and `docs/reference/evidence.md` again. Do not repeat the archive or Twydo-to-iPhone rounds.
- [ ] **Step 2:** Write the checklist and show it to the user.
  - Scope: one new list on the iPhone, **test account only**, synthetic name `Twydo discovery probe`, a non-default color, one open task and one completed task with synthetic titles.
  - Then: phone sync; restart the test daemon, or run `tdo sync --discover`, with `XDG_CONFIG_HOME=~/twydo-phase0/xdg`.
  - Record: the local folder name equals the server collection id; name and color as shown in Twydo; both tasks present; what the phone shows.
  - No deletion, no account or write-mode changes, no production data.
- [ ] **Step 3: STOP.** Ask for explicit approval before any phone or NAS action.
- [ ] **Step 4 (after approval and a passing round):**
  - Capture `fixtures/list-create/phone-created-discovery/` with `tools/capture_fixture.py`, with a `case.toml` whose `claim`, `observed`, `tests` and `supports = ["list-create.collection"]` match the result.
  - Set `iphone_to_twydo = "pass"`, add the case to `evidence`, and update `notes`.
  - Update `docs/reference/evidence.md` and `docs/compatibility.md`.
  - Run `python3 tools/check_evidence.py`, `python3 tools/check_links.py` and `cargo test --workspace`.
  - Commit `feat(compat): verify phone-created list discovery (#76)`.
