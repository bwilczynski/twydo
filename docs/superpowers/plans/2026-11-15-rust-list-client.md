# Rust List Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the bundled Python helper that wraps vdirsyncer's internals; Twydo performs
list operations against the CalDAV server in Rust and uses vdirsyncer only as a command.

**Architecture:** The helper process becomes in-process calls. `lists/dav.rs` holds the server
calls, `lists/engine.rs` holds what the helper's `main`, guards and `dispatch` did, and
`sync/vdirsyncer.rs` is the only code that knows vdirsyncer's command line. Tests run against
a fake CalDAV server in a new test-support crate.

**Tech Stack:** Rust (workspace edition as is), `ureq` 3 with rustls, `roxmltree`, std only
for the fake server.

**Spec:** [docs/superpowers/specs/2026-11-15-rust-list-client-design.md](../specs/2026-11-15-rust-list-client-design.md)

**Reference for the port:** the helper as of commit `b7da846`. After Task 6 deletes it, read
it with `git show b7da846:crates/twydo-core/src/lists/helper.py`. Where this plan says "as the
helper does", that file is the specification: same request bodies, headers, status checks,
limits and refusal codes. vdirsyncer's own discovery, which the home lookup replaces, is in
`/usr/lib/python3.14/site-packages/vdirsyncer/storage/dav.py` (class `Discovery`).

## Global Constraints

- No Twydo code imports, embeds or runs Python at runtime. No subprocess, JSON protocol or
  interpreter setting replaces the helper.
- The only dependency on vdirsyncer is its command line: `sync`, `metasync`, `discover`.
- No path may create a list on the server during discovery. The generated vdirsyncer config
  never contains `implicit`.
- Guards are not weakened: where this plan is silent on a check the helper makes, keep it.
- Every request stays on the configured origin, follows no redirect, and treats 3xx as failure.
- Response body cap 4 MiB (`4 * 1024 * 1024`); delete backup cap 32 MiB in total.
- Error messages never contain server output, addresses or credentials.
- Operation records stay `format: 1` and records written by the current version stay readable.
- New dependencies must be permissively licensed (MIT, Apache-2.0, ISC, BSD).
- No device, NAS or production access. Tests use the fake server only.
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`. Cargo target dirs never go in `/tmp`.
- Commits pass commitlint (`type(scope): subject`, lowercase subject start, header ≤ 100,
  body lines ≤ 100). No agent attribution, no `Co-Authored-By` trailer.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --check` pass at the end of every task.

## Review Focus

1. The server answers with a redirect or an `href` on another origin: the operation fails
   with the `dav` wording and nothing is sent to the other origin. (Task 2)
2. `password_command` fails or prints nothing: the operation fails with the `credentials`
   wording before any request, and the record shows no remote write. (Task 4)
3. The home lists a collection whose id is not a plain id (leading dot, `%`, `/`): discovery
   fails with the `mapping` wording and creates no folder. (Task 5)
4. The daemon dies between `remote-started` and the outcome: the next reconcile resolves the
   record and no second `MKCALENDAR` or `DELETE` is sent. (Task 6, existing recovery tests)
5. A config still containing `list_python`: loading fails with the `run tdo config init`
   hint and `tdo config init` removes the line. (Task 7)

## File Structure

| File | Responsibility |
|---|---|
| `crates/twydo-testdav/` (new, `publish = false`) | Fake CalDAV server and fake `vdirsyncer` script for tests |
| `crates/twydo-core/src/lists/dav.rs` (new) | Connection and the CalDAV calls; no records, no locks |
| `crates/twydo-core/src/lists/call.rs` (new) | `RemoteRequest`, `Refused`, refusal wordings |
| `crates/twydo-core/src/lists/engine.rs` (new) | Locks, record guards, phases, dispatch |
| `crates/twydo-core/src/sync/vdirsyncer.rs` | Config generation plus every vdirsyncer invocation |
| `crates/twydo-core/src/sync/discover.rs` | Discovery, rewritten |
| deleted | `lists/helper.rs`, `lists/helper.py`, `sync/list_python.rs`, `tests/lists_helper.rs`, `tests/common/list_backend.py`, `tools/tests/test_list_helper.py` |

---

### Task 1: Test-support crate with a fake CalDAV server

**Files:**
- Create: `crates/twydo-testdav/Cargo.toml`, `crates/twydo-testdav/src/lib.rs`
- Modify: root `Cargo.toml` (workspace member)
- Test: `crates/twydo-testdav/tests/server.rs`

**Interfaces:**
- Produces:
  - `pub struct FakeDav` with `pub fn start(root: &Path) -> FakeDav`, `pub fn url(&self) -> String`
    (`http://127.0.0.1:<port>/dav/alice/`, the value for `[sync].url`). Dropping it stops the
    server.
  - `pub fn add_collection(root: &Path, id: &str, name: &str, color: &str)`
  - `pub fn collections(root: &Path) -> BTreeMap<String, (Option<String>, Option<String>)>`
  - `pub fn flag(root: &Path, name: &str)`, `pub fn flag_value(root: &Path, name: &str, value: &str)`
  - `pub fn calls(root: &Path) -> Vec<String>`
  - `pub fn fake_vdirsyncer(root: &Path) -> PathBuf` (writes an executable `sh` script)

State lives in files under `root` so a daemon in another process and the shell script see it:

- `server.json`: `{"collections": {id: {"name", "color", "resources": {name: {"etag", "body"}}}}}`.
- `flags/<name>`: presence means set; content is the value. `_once` flags are deleted when used.
- `calls.log`: one line per request method (`PROPFIND`, `MKCALENDAR`, ...) and `DISCOVER`,
  `SYNC`, `METASYNC` from the script, appended with `O_APPEND`.

Server behaviour:

- Hand-written HTTP/1.1 on `127.0.0.1:0`, one thread, `Connection: close`. Requires basic
  auth `alice:secret`; otherwise 401.
- `PROPFIND` on the base URL answers `current-user-principal` (`/dav/principals/alice/`); on
  the principal answers `calendar-home-set` (`/dav/alice/`); depth 1 on the home lists each
  collection with `resourcetype` (`collection`, `calendar`).
- On a collection: `PROPFIND` (named, `allprop`, depth 1 inventory with strong etags), `GET`
  with `If-Match`, `PROPPATCH`, `MKCALENDAR` (201; 405 if it exists), `DELETE` (204); 404
  when absent.
- Flags, with the meaning the Python fake gave them: `fail_status`, `ignore_write`,
  `fail_delete_once`, `partial_delete_once`, `mutate_before_delete`, `inventory_fault`
  (values as in `tools/tests/test_list_helper.py` at `b7da846`), `hang_after_create_once`,
  `hang_after_delete_once`, `hang_seconds`, plus `redirect` (every answer is a 302 to
  `http://127.0.0.1:1/`) and `oversize` (a 5 MiB body).
- The script: appends `DISCOVER`/`SYNC`/`METASYNC` by first argument; for `discover`, honours
  `fail_refresh_once` (exit 1), `discover_sleep` and `hang_after_adopt` (sleep), and exits 1
  with `error: Unable to find or create collection` on stderr when a local folder (from
  `$VDIRSYNCER_CONFIG`'s sibling state, passed as env `TWYDO_TEST_VDIR`) has no entry in
  `server.json` or the reverse; otherwise writes `status/twydo.collections` next to the config
  and exits 0.

- [ ] **Step 1: Write failing tests** in `tests/server.rs`, speaking raw HTTP over `TcpStream`:
  `unauthenticated_request_is_401`, `home_lists_added_collections`,
  `mkcalendar_then_propfind_reads_name_and_color`, `delete_removes_the_collection`,
  `calls_are_logged_in_order`, `fake_vdirsyncer_discover_fails_on_a_one_sided_list`.
- [ ] **Step 2: Run** `cargo test -p twydo-testdav`; expected: compile failure.
- [ ] **Step 3: Implement** the crate.
- [ ] **Step 4: Run** `cargo test -p twydo-testdav`; expected: all pass.
- [ ] **Step 5: Commit** `test(lists): add a fake caldav server for tests`

### Task 2: CalDAV calls

**Files:**
- Create: `crates/twydo-core/src/lists/dav.rs`, `crates/twydo-core/src/lists/call.rs`
- Modify: `crates/twydo-core/src/lists.rs` (`mod dav; mod call;`), `crates/twydo-core/Cargo.toml`
  (`ureq`, `roxmltree`; dev-dependency `twydo-testdav`)
- Test: `crates/twydo-core/tests/lists_dav.rs` (expose what it needs with `#[doc(hidden)] pub`)

**Interfaces:**
- Produces, in `call.rs`:
  - `pub struct Refused { pub code: &'static str, pub missing: Vec<String>, pub sent: bool }`
    (`sent: false` is the helper's `NotSent`); `Refused::new(code)`.
  - `pub fn wording(code: &str) -> &'static str`: the `known_refusal` table from
    `helper.rs`, without `dependency` and `protocol`; `credentials` reads
    `"collection credential fetch failed; check [sync].password_command"`.
- Produces, in `dav.rs`:
  - `pub struct Dav` and `pub fn connect(sync: &SyncConfig, deadline: Instant) -> Result<Dav, Refused>`
    (runs `password_command`; `credentials` on failure or empty output).
  - `pub fn home(&self) -> Result<String, Refused>`
  - `pub fn list_home(&self) -> Result<Vec<Listed>, Refused>`, `pub struct Listed { pub id: String, pub url: String }`
  - `pub fn collection(&self, url: &str, id: &str) -> Result<Collection<'_>, Refused>` (runs
    `validate_target`)
  - on `Collection`: `read_properties() -> Result<Properties, Refused>`
    (`Properties { name: Option<String>, color: Option<String> }`), `absent() -> Result<bool, Refused>`,
    `create(name, color) -> Result<Properties, Refused>`,
    `write_metadata(changes: &Changes, expected: &Properties) -> Result<Properties, Refused>`,
    `capture() -> Result<Captured, Refused>` (serializes to the `remote.json` shape the helper
    writes), `delete(backup: &Captured) -> Result<(), Refused>`,
    `reconcile_delete(backup: &Captured) -> Result<(), Refused>`
  - `pub fn canonical_color(value: &str) -> Option<String>`, `pub fn validate_target(base, target, id) -> Result<(), Refused>`

Decisions:

- Check `ureq`'s current docs for the feature names; requirements are rustls, the system
  trust store by default, `ca_file` as the only root when set, redirects disabled, a
  per-request timeout of `min(30 s, time left to deadline)`, and non-2xx statuses returned as
  responses, not errors.
- User agent `tdo/<CARGO_PKG_VERSION>`. Basic auth header built by hand.
- Request XML is written as strings with an escaping function; responses parsed with `roxmltree`.
- Home lookup follows vdirsyncer's `Discovery`: `list_home` first lists the configured URL at
  depth 1 and uses its calendars if any; otherwise principal, then `calendar-home-set`, then
  depth 1 there. `home()` is the `find_home` path only. An id is the percent-decoded last
  path segment; a non-plain id is `Refused("mapping")`.
- Every `Refused` code and every limit matches the helper's function of the same name.

- [ ] **Step 1: Write failing tests** against `FakeDav`:
  `reads_name_and_color`, `create_sends_if_none_match_and_reads_back`,
  `create_refuses_an_existing_collection` (`exists`, no `MKCALENDAR` in `calls`),
  `write_metadata_refuses_a_stale_expectation` (`stale`, no `PROPPATCH`),
  `ignored_write_is_a_dav_refusal` (flag `ignore_write`),
  `capture_matches_what_delete_revalidates`, `delete_refuses_a_changed_inventory`
  (flag `mutate_before_delete`; `changed`, `sent == false`, no `DELETE`),
  `inventory_faults_are_refused` (each `inventory_fault` value gives `inventory`),
  `reconcile_delete_reports_present_partial_and_absent`,
  `list_home_finds_collections_through_the_principal`,
  `a_redirect_is_refused_and_not_followed` (flag `redirect`; `dav`),
  `an_oversized_body_is_refused` (flag `oversize`; `dav`),
  `a_failing_password_command_is_a_credentials_refusal` (`["false"]`; `calls` empty),
  `colors_compare_case_insensitively_and_ignore_alpha`,
  `targets_outside_the_origin_or_with_dot_segments_are_refused`,
  `error_text_never_contains_the_url` (assert `wording(code)` for every code above has no `http`).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test lists_dav`; expected: compile failure.
- [ ] **Step 3: Implement** `call.rs` and `dav.rs`.
- [ ] **Step 4: Run** the same command; expected: all pass.
- [ ] **Step 5: Commit** `feat(lists): add a rust caldav client for list calls`

### Task 3: The vdirsyncer boundary

**Files:**
- Modify: `crates/twydo-core/src/sync/vdirsyncer.rs`, `crates/twydo-core/src/sync/run.rs`

**Interfaces:**
- Produces, in `sync/vdirsyncer.rs`:
  - `pub(crate) trait Control { fn started(&self, _pid: u32) {} fn finished(&self) {} fn stop_requested(&self) -> bool { false } }`
    and `pub(crate) struct Uncontrolled;`
  - `pub(crate) struct Vdirsyncer<'a> { pub sync: &'a SyncConfig, pub config: &'a Path, pub vdir: &'a Path, pub state: &'a Path, pub hook_log: Option<&'a Path> }`
  - `pub(crate) fn discover(&self, c: &dyn Control) -> Result<Ran, SyncError>`,
    `sync(&self, force: &[String], c)`, `metasync(&self, c)`; `Ran` and `Outcome` move here
    unchanged.

Move the body of `run_command` (stdin is the shared vdir lock, own process group, timeout,
stop, output readers) into a private `run` on `Vdirsyncer`. `run.rs` keeps the running-record
bookkeeping by implementing `Control`, and builds no argument list itself: after this task
`grep -n '"discover"\|"metasync"\|"sync".to_string\|--force-delete\|VDIRSYNCER_CONFIG' crates/twydo-core/src/sync/run.rs`
prints nothing. Behaviour does not change.

- [ ] **Step 1: Refactor** as above.
- [ ] **Step 2: Run** `cargo test --workspace`; expected: all pass, no test changed.
- [ ] **Step 3: Run** the `grep` above; expected: no output.
- [ ] **Step 4: Commit** `refactor(sync): put every vdirsyncer invocation behind one type`

### Task 4: The engine

**Files:**
- Create: `crates/twydo-core/src/lists/engine.rs`
- Modify: `crates/twydo-core/src/lists/call.rs`, `crates/twydo-core/src/lists.rs`
- Test: `crates/twydo-core/tests/lists_engine.rs`

**Interfaces:**
- Consumes: Task 2's `Dav`, `Refused`, `wording`; Task 3's `Vdirsyncer::discover`.
- Produces, in `call.rs`: `pub struct RemoteRequest` with the fields of `HelperRequest` minus
  `version`, `pair`, `local_storage` and `config`, same serde names and defaults, plus
  `RemoteRequest::new(cfg: &Config, action: &str) -> Result<Self, ListError>` (what `probe`
  did). It must deserialize a request stored by the current version.
- Produces, in `engine.rs`:
  - `pub(crate) fn run(cfg: &Config, request: &RemoteRequest) -> Result<serde_json::Value, Refused>`
  - `pub(crate) fn invoke(cfg: &Config, request: &RemoteRequest) -> Result<serde_json::Value, ListError>`
    (`ListError::Refused(wording(code).into())`)

`run` is the helper's `main` plus `operation_guard` plus `dispatch` for the actions `resolve`,
`create`, `edit`, `preview-delete`, `delete` and the modes `execute`, `refresh`, `reconcile`:

- Validate the request as `validate_request` does (failures are `Refused("operation")`).
- Take `run_lock(&state.join("sync"), false)` then `VdirLock::shared(&state)` and hold both to
  the end.
- Check the record exactly as `operation_guard` does, using `operation::read`, `snapshot` and
  the phase table (`execute`: `prepared`; `refresh`: `remote-confirmed`; `reconcile`:
  `remote-started`, `remote-confirmed`, `uncertain`).
- Write phases with `operation::save`: `remote-started` before the first server call in
  `execute`; then `remote-confirmed` with the result as `receipt`, or `local-confirmed` after
  a `refresh`; on any failure `uncertain` with the receipts the helper writes
  (`no_remote_write` with `reason`; `present` with `reason`; `observed` for `choice`).
- `refresh` runs `Vdirsyncer::discover` with `Uncontrolled`; a non-zero exit or an
  `error: `/`critical: ` stderr line is `Refused("discovery")`.
- Record `remote_target` in `execute`; `reconcile` uses it after `validate_target`.
- Connection digest: new field `connection: Option<String>` on `ListOperation`
  (`#[serde(default)]`), the SHA-256 hex of `url`, `username`, `password_command`, `ca_file`
  and the canonical vdir, JSON-encoded in that order. Written in `execute`; in the other modes
  a present and different value is `Refused("mapping")`. A record without it (written before
  this change) is not checked. The old `connection_digest` field stays readable and unused.
- Deadline: `Instant::now() + timeout_secs`, passed to `connect`; checked before each server
  call; expiry is `Refused("timeout")` after writing `uncertain`.
- `preview-delete` writes `lists/backups/<record id>/remote.json` with the directory checks
  and fsyncs the helper performs.
- When `TWYDO_TEST_REMOTE_DIR` is set (`vdirsyncer::TEST_REMOTE_ENV`), `run` is
  `Refused("mapping")`, as the helper refuses a non-CalDAV remote today.

The engine is not wired in yet; the helper still serves production calls after this task.

- [ ] **Step 1: Write failing tests**, preparing records with `operation::prepare` (adapt its
  parameter type in Task 6, not here: build the record through a small test helper that
  writes the JSON): `create_walks_prepared_to_remote_confirmed`,
  `refresh_confirms_locally_after_discover` (`DISCOVER` in `calls`),
  `a_stale_edit_records_no_remote_write`, `a_wrong_phase_is_refused_before_any_request`,
  `a_changed_local_collection_is_refused_before_any_request`,
  `a_failing_password_command_records_no_remote_write` (Review Focus 2),
  `a_hung_server_times_out_as_uncertain` (flag `hang_after_create_once`, `timeout_secs = 1`),
  `reconcile_reports_differing_server_values_as_choice`,
  `a_changed_connection_is_a_mapping_refusal`,
  `a_record_from_the_current_version_still_reconciles` (a fixture JSON string containing
  `version`, `pair`, `local_storage`, `config` and `connection_digest`).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test lists_engine`; expected: compile failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same command; expected: all pass.
- [ ] **Step 5: Commit** `feat(lists): run list operations in process`

### Task 5: Discovery

**Files:**
- Modify: `crates/twydo-core/src/sync/discover.rs`, `crates/twydo-core/src/sync/run.rs`,
  `crates/twydo-core/tests/sync_discover.rs`, `crates/twydo-core/tests/common/lists.rs`
- Test: `crates/twydo-core/tests/sync_discover.rs`, `crates/tdo/tests/sync_e2e.rs`

**Interfaces:**
- Consumes: `Dav::connect`, `Dav::list_home`, `Vdirsyncer::discover`, `Control`.
- Produces: `pub fn discover(cfg: &Config, state: &Path) -> Result<Discovered, DiscoverError>`
  and `pub(crate) fn discover_watched(cfg, state, control: &dyn Control)`, same public types
  as today. `DiscoverError::Unavailable` is now only "vdirsyncer cannot be started".

Algorithm (the caller holds the run lock and the shared vdir lock, as today):

1. `stop_requested` → `Stopped`. Test seam set → `Failed(wording("mapping"))`.
2. `list_home`. A `Refused` maps to `Failed(wording(code))`; `timeout` maps to `TimedOut`.
3. Local top-level directories not starting with `.` versus listed ids. The first local id
   (sorted) missing on the server → `Missing { id }`; nothing is created or run.
4. For each listed id with no local directory (sorted): `create_dir`, remember it as adopted.
5. `stop_requested` → remove adopted directories with `remove_dir` (never recursive) → `Stopped`.
6. `Vdirsyncer::discover(control)`. Failure (spawn failure → `Unavailable`; stop → `Stopped`;
   timeout → `TimedOut`; non-zero exit or `error: `/`critical: ` line → `Failed`) removes the
   adopted directories the same way.
7. `Ok(Discovered { adopted })`.

`common/lists.rs::offline()` now starts a `FakeDav`, sets `[sync].url`, `username = "alice"`,
`password_command = ["printf", "secret"]`, `command = fake_vdirsyncer(root)`, and returns the
server guard with the tempdir. `sync_discover.rs`'s `script(remote_only, local_only)` becomes
"add these collections to the server" and "create these local folders".

- [ ] **Step 1: Rewrite the tests** keeping the three existing names and assertions
  (`discover_adopts_a_remote_only_list`, `discover_never_creates_a_remote_list`,
  `discover_maps_other_refusals`) and add `a_failed_vdirsyncer_discover_removes_adopted_folders`
  (flag `fail_refresh_once`), `a_non_plain_server_id_fails_and_creates_nothing`
  (Review Focus 3), `a_stop_before_discover_adopts_nothing`.
- [ ] **Step 2: Add the backstop test** to `crates/tdo/tests/sync_e2e.rs`, skipped when
  `vdirsyncer` is not installed like its neighbours:
  `real_vdirsyncer_discover_without_input_creates_nothing`. With the filesystem test remote, a
  local folder that has no remote folder, and stdin at EOF, run `Vdirsyncer::discover`;
  assert it fails and the remote directory still has no such folder. Add to the config
  rendering tests: the rendered config does not contain `implicit`.
- [ ] **Step 3: Run** `cargo test -p twydo-core --test sync_discover`; expected: failures.
- [ ] **Step 4: Implement**; in `run.rs` replace `DiscoverWatch`'s `Watch` impl with `Control`.
- [ ] **Step 5: Run** `cargo test --workspace`; expected: all pass (`lists_helper` and the
  list tests still use the helper and still pass).
- [ ] **Step 6: Commit** `feat(sync): discover lists without the python helper`

### Task 6: Switch list operations over and delete the helper

**Files:**
- Modify: `crates/twydo-core/src/lists.rs`, `lists/remote.rs`, `lists/backup.rs`,
  `lists/operation.rs`, `crates/twydo-core/src/sync/mod.rs`,
  `crates/twydo-core/src/config.rs` (drop the `list_python` field; Task 7 handles the removed
  key), every test harness that sets `list_python` or reads `server.json` calls or flags:
  `crates/twydo-core/tests/common/{lists.rs,mod.rs}`, `crates/tdo/tests/common/mod.rs`,
  `crates/twydo-daemon/tests/`, and the tests listed in the spec's "Kept" list
- Delete: `crates/twydo-core/src/lists/helper.rs`, `crates/twydo-core/src/lists/helper.py`,
  `crates/twydo-core/src/sync/list_python.rs`, `crates/twydo-core/tests/lists_helper.rs`,
  `crates/twydo-core/tests/common/list_backend.py`, `tools/tests/test_list_helper.py`

**Interfaces:**
- Consumes: `engine::invoke`, `engine::run`, `RemoteRequest`, `wording`.
- Produces: `ListOperation.request: RemoteRequest`; `operation::prepare(cfg, request: RemoteRequest, now)`.

Call-site mapping:

- `helper::invoke(cfg, &req)` → `engine::invoke(cfg, &req)`.
- `helper::exchange(...)` then `reply.ok` / `reply.code` → `engine::run(...)` then
  `Ok(_)` / `Err(refused)` with `refused.code`.
- `helper::refusal(cfg, code)` → `wording(code)`.
- `HelperRequest::probe(cfg)` then setting `action` → `RemoteRequest::new(cfg, action)`.
- `operation::recovery_request` no longer refuses a record for its stored config path and no
  longer sets `config`; it still ensures the generated vdirsyncer config under the run lock.

Tests: assertions stay; only harness plumbing and wording that named the helper change. If a
kept test's assertion cannot hold without the helper process (for example one about the
helper's interpreter or its stdout), delete that test and list it in the commit body.
`fixtures/list-create/phone-confirmation/case.toml` names
`lists_remote.rs::actual_service_and_bundled_helper_create_edit_and_confirm_local_discovery`:
rename the test to `actual_service_creates_edits_and_confirms_local_discovery` and update the
fixture's `tests` entry.

- [ ] **Step 1: Switch** the harnesses to `FakeDav` and `fake_vdirsyncer`; switch the call sites.
- [ ] **Step 2: Delete** the files above and their `mod` lines.
- [ ] **Step 3: Run** `cargo test --workspace` and `python3 -m unittest discover -s tools/tests -t .`;
  expected: all pass.
- [ ] **Step 4: Run** `git grep -n -i "helper\.py\|list_python\|list_backend\|HelperRequest\|TWYDO_HELPER_REQUEST\|python" -- crates`;
  expected: only `list_python` inside the removed-key handling added in Task 7 (none yet) and
  unrelated uses of the word in comments about vdirsyncer being a Python program; no `.py`
  file remains under `crates/`.
- [ ] **Step 5: Commit** `feat(lists)!: remove the python helper around vdirsyncer`

### Task 7: Configuration, checks and schema

**Files:**
- Modify: `crates/twydo-core/src/config.rs`, `crates/tdo/src/cli/config/check.rs`,
  `crates/tdo/src/cli/config/init.rs`, `crates/tdo/tests/config_check.rs`,
  `crates/twydo-core/tests/config.rs`, `schema/tdo.openrpc.json` (regenerated; leave
  `schema/released.openrpc.json` alone)

Decisions:

- `REMOVED_SYNC_KEYS` gains `"list_python"`; the constant's length and the removed-keys
  sentence in the error follow.
- `tdo config init` removes a `list_python` line. The existing test
  `old_production_shape_is_cleaned` moves `list_python = "python3"` from its `keep` list to
  its `gone` list.
- `tdo config check` drops the `list helper` step and its hints. No replacement step.
- Regenerate the schema with the project's existing command (see `docs/testing.md`).

- [ ] **Step 1: Write failing tests:** `a_leftover_list_python_fails_with_the_init_hint` in
  `crates/twydo-core/tests/config.rs` (error text contains `list_python` and
  `tdo config init`); update `old_production_shape_is_cleaned`; in `config_check.rs` delete
  the `list helper` cases and assert the step list has no `list helper` entry.
- [ ] **Step 2: Run** `cargo test --workspace`; expected: the new and changed tests fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace`; expected: all pass, including the schema
  freshness test.
- [ ] **Step 5: Commit** `feat(config)!: remove the list_python setting`

### Task 8: Documentation

**Files:**
- Modify: `docs/sync.md`, `docs/config.md`, `docs/install.md`, `docs/architecture.md`,
  `docs/testing.md`, `AGENTS.md`

Content rules:

- No page describes a helper, a Python interpreter or a vdirsyncer version pin. Runtime
  requirement: the `vdirsyncer` command.
- `docs/sync.md` discovery section: Twydo lists the server itself, adopts new lists by
  creating empty local folders, stops at a local list that is gone from the server, then
  runs `vdirsyncer discover` with no terminal input so nothing can be created on the server.
  Stopping takes effect between server requests.
- `docs/architecture.md`: list operations run in the daemon process, which holds the run lock
  and the shared vdir lock itself; a killed daemon leaves `remote-started`, which is
  reconciled as uncertain.
- `docs/config.md`: `list_python` moves to the removed-keys sentence; the `list helper` row
  leaves the check table; the PATH note stays for `command` only.
- `docs/testing.md`: the fake CalDAV server crate replaces the Python fake.
- `AGENTS.md`: under "Current work and roadmap", state that #86 phase 1 is implemented and
  that the server check in the spec is pending and needs the user's approval before release.
- Do not edit `CHANGELOG.md`; the release tooling writes it.

- [ ] **Step 1: Edit** the pages.
- [ ] **Step 2: Run** `python3 tools/check_links.py`; expected: exit 0.
- [ ] **Step 3: Run** `git grep -n -i "list_python\|bundled.*helper\|collection helper\|0\.21" -- docs/*.md AGENTS.md`;
  expected: only the removed-keys sentence in `docs/config.md`.
- [ ] **Step 4: Commit** `docs: describe list operations without the python helper`

## After the last task

- Run `npx --yes -p @commitlint/cli -p @commitlint/config-conventional commitlint --from=origin/main --to=HEAD`.
- Do not push, open a pull request or contact any server. The spec's server check needs the
  user's approval first.
