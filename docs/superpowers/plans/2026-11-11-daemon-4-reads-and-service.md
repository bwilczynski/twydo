# Daemon plan 4: read-only commands, the service unit and the docs. Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After this plan, no `tdo` client opens the vdir, the state directory or `config.toml`.
`check`, `export-json`, `status`, `conflicts`, `conflicts show` and `op check` go through the
daemon. `tdo daemon install`, `uninstall` and `status` manage a systemd user service or a launchd
agent. Clients start the daemon through that unit when one is installed. The TUI warns when no
unit is installed, and the contrib timer units are replaced.

**Architecture:** The plan has three parts:

- **Tasks 1–4 move the read-only commands.** Each one gets a daemon method that returns what
  the command prints today, then the command becomes a thin client. Task 4 deletes the
  file-reading helpers from `tdo` and adds a source scan that keeps them out.
- **Tasks 5–8 add the service unit.** A new `crates/tdo/src/service/` module names, renders,
  finds and drives the unit. Task 5 builds it as pure functions plus a command seam. Task 6
  adds the commands, Task 7 the start, stop and upgrade paths, and Task 8 the warning.
- **Task 9 writes the documentation.**

**Tech Stack:** Rust 2024; serde, `schemars` 1.x, `libc`, the crates of plans 1 to 3. No new
crates. The plist is written as text (no plist crate).

**Spec:** [`docs/superpowers/specs/2026-11-09-daemon-design.md`](../specs/2026-11-09-daemon-design.md)
(§6.2, §6.8, §7.5, §10.1–§10.4, §12 step 2, §13 "Install" and "Lifecycle", §14). Plans 1 to 3
describe the crate layout, the lanes, replay and the client connect path:
[`2026-11-09-daemon-1-core-and-writes.md`](2026-11-09-daemon-1-core-and-writes.md),
[`2026-11-10-daemon-2-sync-and-jobs.md`](2026-11-10-daemon-2-sync-and-jobs.md),
[`2026-11-10-daemon-3-clients-switch.md`](2026-11-10-daemon-3-clients-switch.md).

## Where this plan sits

| Plan | Delivers | Clients |
|---|---|---|
| 1–3 (merged, PR #78–#80) | daemon, protocol, writes, sync, jobs; every changing client over the socket | read-only commands still read files |
| **4 (this)** | read-only commands through the daemon; `tdo daemon install`/`uninstall`/`status`; service-manager start; no-unit warning; contrib units; docs | **nothing reads files** (D9) |
| next | write-queue plan 3 (rebasing in the daemon), then Plan 2 (the notifier) | |

## Decisions this plan makes (builder's calls in the spec)

- **One report method for `check` and `op check`.** `check.report {verbose, matrix?, uid?}`
  returns `{lines, warnings}`: the exact stdout lines and stderr warnings of today's command.
  With `uid` it returns today's `tdo op check` lines. With `matrix` (an absolute path the client
  resolved) the daemon loads that file for this one report. A file that does not load is
  refused `invalid`. So the visible `tdo check --matrix` keeps working, and the hidden
  `tdo op check --matrix` test hook stays. Lines rather than a structured report: the report is
  for people, and no other front end needs it yet.
- **`export` returns the schema-v1 document**, as `{skipped, document}`, built from the vdir as
  last read (the publisher's base report, **without** the queue overlay). That matches today,
  where `export-json` never applied the queue. `check.report` uses the published (overlaid)
  snapshot, as today's `permission_snapshot` does. The `X-2DO*` property names are read from the
  files on the request thread, never on the vdir lane.
- **`conflicts.show {id}`** returns structured detail: four fields (`SUMMARY`, `DUE`, `STATUS`,
  `LAST-MODIFIED`) for each of local, remote and current, plus absolute file paths. The client
  prints today's layout.
- **`tdo status`** is built from `snapshot.get` (configuration warnings, timezone),
  `status.get` and `conflicts.list`. `pending:` prints `journal.unfinished`, which is what
  `SyncSummary::pending` was. Its output is otherwise unchanged.
- **Unit names.** With the default socket: `twydo.service` and the launchd label
  `app.twydo.daemon`. With any other socket: `twydo-<socket-id>.service` and
  `app.twydo.daemon.<socket-id>`, where `<socket-id>` is `paths::socket_id`. Unit files:
  `$XDG_CONFIG_HOME/systemd/user/` (else `$HOME/.config/systemd/user/`) on Linux, and
  `$HOME/Library/LaunchAgents/<label>.plist` on macOS. "Installed" means that file exists
  (spec §10.1).
- **What the unit passes.** `ExecStart=<stable tdo> daemon`, plus `--vdir <abs>` when given.
  On Linux, `--socket <abs>` only when the socket is not the default: the user manager sets
  `XDG_RUNTIME_DIR`. On macOS `--socket` is always passed, because launchd agents do not get
  the shell's `TMPDIR`. Environment: `PATH` as the installing shell has it (the daemon runs
  vdirsyncer and the conflict hook), and each of `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and
  `XDG_DATA_HOME` that is set. This is wider than the spec's "when not the default", because a
  service manager's environment is not the shell's.
- **Stable `tdo` path.** The candidates, in order: each `PATH` entry's `tdo`, then
  `${MISE_DATA_DIR:-$HOME/.local/share/mise}/shims/tdo`. The first that exists and has no
  `/installs/` component in its path wins. `mise activate` puts the versioned install directory
  on `PATH`, so skipping it is required. If no candidate is left, `install` refuses with
  `no stable path to tdo (found only <path>); run 'mise reshim' or pass --exe <path>`. `--exe`
  overrides the search, unchecked.
- **systemd unit:** `Type=simple`, `Restart=always`, `RestartSec=1`,
  `RestartPreventExitStatus=10` (`EXIT_ALREADY_RUNNING`), `TimeoutStopSec=90` and
  `WantedBy=default.target`. 90 s covers the 10 s shutdown grace, the stop of a running
  vdirsyncer and the deferred plan-2 item "the unit's stop timeout". **launchd agent:**
  `RunAtLoad`, `KeepAlive` true, `ExitTimeOut` 90, and `StandardErrorPath` set to the detached
  daemon's log path (`client::start::log_path`). launchd cannot exclude one exit code, so a lock
  loser is restarted every 10 s (launchd's throttle) until it wins or the unit is removed.
  Install stops a detached daemon first, so this happens only in a race.
- **The service-manager seam is `PATH`.** The code runs `systemctl` and `launchctl` by name.
  Tests put fake scripts first on `PATH` that append their arguments to a log. Every test that
  can reach this code sets `XDG_CONFIG_HOME` (and `HOME`) to tempdirs, so no real unit is ever
  found or written.
- **Starting through the unit.** With a unit installed, a client that finds no daemon runs
  `systemctl --user start <unit>`, or `launchctl bootstrap gui/<uid> <plist>` (an "already
  loaded" failure is ignored) and then `launchctl kickstart gui/<uid>/<label>`. If that command
  fails, the client falls back to the detached start and keeps the manager's error for the
  final message if nothing answers.
- **`tdo daemon stop` with a unit installed** stops through the manager (`systemctl --user stop`,
  `launchctl bootout gui/<uid>/<label>`), since `Restart=always` and `KeepAlive` would otherwise
  bring the daemon straight back. Without a unit, it is today's `daemon.shutdown`.
- **Upgrade under a unit.** After `daemon.shutdown`, the service manager may restart the daemon
  before the client sees the socket go. The client's wait therefore ends when the socket is
  gone **or** a daemon with a different epoch answers (`client::wait_replaced`).
- **The old timer.** `install` disables `twydo-sync.timer` (`systemctl --user disable --now`) or
  `app.twydo.sync` (`launchctl bootout` and `launchctl disable`) when its file is found, says so,
  and leaves the files in place. `tdo sync --background` and `--when-due` stay, hidden, so an
  installed timer is harmless until then.
- **The warning switch** is a new `config.toml` table, `[daemon]`, with `unit_warning = true`
  as the default. `Config` uses `deny_unknown_fields`, so a client one release older would fail
  to decode a wire `config` that has the new key. The field is therefore
  `#[serde(skip_serializing)]` on `Config`, and the snapshot carries it as
  `WireSnapshot.unit_warning: bool`, with `#[serde(default = "yes")]` for older daemons. The
  message: `daemon won't start at login; run tdo daemon install`.
- **`tdo daemon schema`** prints `schema::document()` without a daemon. The spec §6.2 note
  "not built yet" is removed.
- **No sync-interval key.** The periodic interval stays 60 s. The spec leaves the key to the
  builder, and nobody has asked for it. `docs/config.md` says it is fixed.

## Global Constraints

- No personal data in the repository: tests use tempdirs and synthetic tasks (AGENTS.md).
- Integration tests build every `tdo` through `common::tdo_bin`/`TestEnv::tdo_cmd`, so
  `TWYDO_SOCKET`, `XDG_CONFIG_HOME` and `XDG_STATE_HOME` are always set (spec §10.4). Tests that
  reach the service code also set `HOME` and put the fake `systemctl`/`launchctl` first on `PATH`.
- No test, and no step of this plan, runs the real `systemctl`/`launchctl` or writes under the
  user's `~/.config/systemd`. Running `tdo daemon install` on the user's machine needs the
  user's go-ahead (AGENTS.md: production setup).
- New methods follow spec §6.8 naming. The schema is regenerated (`schema/tdo.openrpc.json`),
  and the breaking-change test against `schema/released.openrpc.json` still passes: only
  additions.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` are green
  at every commit. Build into `~/.cache`, never `/tmp` (the cargo target lives outside the
  quota-limited tmpfs).
- Messages keep today's wording. Exit codes: 0 ok, 1 blocked or not found where today's
  command uses 1, 2 usage, config, IO or daemon errors.
- `python3 tools/check_links.py` passes after any docs change.

## Review Focus

1. **An older client meets the new snapshot.** `Config` is `deny_unknown_fields`, so the
   `[daemon]` key must never appear in the wire `config`, and a snapshot without
   `unit_warning` must decode as `true` (Task 8).
2. **A unit started by the service manager has a bare environment.** Without the shell's
   `PATH` and `XDG_*`, the daemon cannot find vdirsyncer or reads another configuration.
   The rendered unit must carry them (Task 5).
3. **An upgrade under `Restart=always`.** systemd brings the new daemon up before the client
   sees the socket go. The client must accept a new epoch as "replaced" instead of reporting
   "the older daemon is still busy" after 30 s (Task 7).
4. **`tdo daemon stop` with a unit installed.** It must go through the manager, or the service
   restarts the daemon at once and `stop` reports success while a daemon runs (Task 7).
5. **Install while a detached daemon runs.** The service's daemon would lose the lock, exit
   10 and stay down. Install must stop the detached daemon first and check that the service's
   daemon answers (Task 6).

---

### Task 1: `check.report`; `tdo check` and `tdo op check` through the daemon

**Files:**
- Create: `crates/twydo-daemon/src/server/reports.rs` (the `check.report` handler and the report
  builder moved from `crates/tdo/src/cli/check.rs`)
- Move: `crates/tdo/src/cli/permissions.rs` → `crates/twydo-daemon/src/server/permissions.rs`
  (with its unit tests; `pub(crate)` instead of `pub(super)`)
- Create: `crates/twydo-daemon/src/protocol/reports.rs` (params and results of Tasks 1 and 2)
- Modify: `crates/twydo-daemon/src/protocol/mod.rs`, `server/mod.rs`, `server/dispatch.rs`,
  `schema.rs`, `schema/tdo.openrpc.json`
- Modify: `crates/tdo/src/cli/check.rs`, `crates/tdo/src/cli/op.rs`, `crates/tdo/src/main.rs`
- Test: `crates/twydo-daemon/tests/reports.rs`, `crates/tdo/tests/cli.rs`, `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Produces:
  - `protocol::reports::CheckReportParams { verbose: bool, #[serde(default)] matrix: Option<PathBuf>, #[serde(default)] uid: Option<String> }`
  - `protocol::reports::CheckReport { lines: Vec<String>, warnings: Vec<String> }`
  - method `check.report` (not changing, so no `request_id`)
  - `server::reports::check_report(shared: &Arc<Shared>, p: CheckReportParams) -> Result<Value, RpcError>`
  - test helper `common::VdirEnv::new(vdir: &Path) -> VdirEnv` with `fn tdo(&self, args: &[&str]) -> Output`, in `crates/tdo/tests/common/mod.rs`: no config file, `--vdir <vdir>` prepended, its own socket and state dirs, and the daemon stopped on drop. Tasks 2–4 use it.

- [ ] **Step 1: Write the failing tests.**
  - `twydo-daemon/tests/reports.rs::check_report_matches_the_old_output`: a fixture vdir with
    one open weekly task and one task blocked by the gate. The lines contain `Data health`,
    `tasks: 2`, `parse errors: 0`, a recurrence line with `preserve=` and an
    `Operation availability (mode=test)` section. `warnings` is empty.
  - `check_report_for_one_uid`: with `uid` set, `lines` equals
    `server::permissions::task_lines(&probes(...))` for that task, the last line is the
    "Permission snapshot only; …" sentence, and `warnings` contains
    `test write mode: unverified operations are enabled`.
  - `check_report_unknown_uid_is_not_found`: kind `not_found`, message `no task with UID nope`.
  - `check_report_with_a_missing_matrix_is_invalid`: `matrix: /nonexistent.toml` gives kind
    `invalid`, and the message names the path.
  - `check_report_counts_queued_edits`: a `create` queued while the lane is held (the held-lane
    setup of `crates/twydo-daemon/tests/queue.rs`) shows up as `tasks: 3`, as today's overlaid
    permission snapshot counts it.
  - In `crates/tdo/tests/cli.rs`, move every `check` test (`check_reports_rules_without_titles`
    through `duplicate_and_cancelled_resources_are_not_blanket_permissions`, plus
    `check_finishes_permissions_for_a_large_collection`) onto `VdirEnv`, keeping each
    assertion. `check_exit_code` and `invalid_config_exits_2` keep asserting exit 2, and also
    assert that stderr names the vdir or the config error (the daemon's last log line).
  - Add `check_leaves_the_fixture_untouched`: listing the sample vdir recursively (names, sizes
    and mtimes) before and after `tdo check` gives the same result.
  - In `cli_op.rs`, move the `op check` tests onto the daemon env: an unknown UID exits 1 with
    `tdo: no task with UID …`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test reports` and
  `cargo test -p tdo --test cli --test cli_op`. Expected: unknown method `check.report`; the
  `tdo` tests fail to compile until `VdirEnv` exists, then fail on the method.
- [ ] **Step 3: Implement the daemon side.** Move `permissions.rs` and turn `cli/check.rs::run`'s
  printing into `fn check_lines(snap: &Snapshot, matrix: &Matrix, verbose: bool, now: DateTime<Utc>) -> Vec<String>`
  in `server/reports.rs`, the same lines in the same order. The handler takes the published
  snapshot (`shared.publisher.current().1`), uses the daemon's matrix unless `matrix` is given,
  and runs on the request thread.
- [ ] **Step 4: Switch the clients.** `tdo check` connects (`sync::connect`), calls `check.report`
  with `matrix` made absolute (`std::path::absolute`), prints `warnings` to stderr as
  `tdo: <w>` and `lines` to stdout, and exits 0. `tdo op check` does the same with `uid`, maps
  `not_found` to exit 1, and maps every other error through `Failure`.
- [ ] **Step 5: Regenerate the schema and run everything.** Regenerate the schema as the
  schema test describes, then run `cargo test -p twydo-daemon -p tdo`. All green.
- [ ] **Step 6: Commit.** `feat(daemon): check.report; tdo check and op check ask the daemon`

### Task 2: `export`; `tdo export-json` through the daemon

**Files:**
- Modify: `crates/twydo-daemon/src/server/reports.rs` (move `task_json`, `export`, `x2do_props`
  and the date helpers from `crates/tdo/src/cli/export.rs`), `protocol/reports.rs`,
  `server/publish.rs`, `dispatch.rs`, `schema.rs`, `schema/tdo.openrpc.json`
- Modify: `crates/tdo/src/cli/export.rs` (now only the client)
- Test: `crates/twydo-daemon/tests/reports.rs`, `crates/tdo/tests/cli.rs`

**Interfaces:**
- Consumes: `VdirEnv` (Task 1).
- Produces:
  - `protocol::reports::ExportResult { skipped: usize, document: serde_json::Value }`. The
    `document` schema is "any", described as "schema v1, as read by tools/reconcile.py".
  - method `export` (params `{}`)
  - `Publisher::disk_snapshot(&self) -> Snapshot`: the last reload's report and the live
    configuration, without the queue overlay.

- [ ] **Step 1: Write the failing tests.**
  - `reports.rs::export_matches_schema_v1`: the document has `schema: 1`, collections sorted by
    id with `#RRGGBB` colours, tasks sorted by `(collection, uid)`, and `x2do_props` listing an
    `X-2DOAPP-…` property that the fixture file carries.
  - `export_ignores_queued_edits`: a title edit queued behind a held lane is not in the
    document (the old title is).
  - `export_counts_unparseable_files`: a broken `.ics` gives `skipped: 1`.
  - `cli.rs`: `export_json_sample` and `config_supplies_vdir_and_timezone` run through
    `VdirEnv` (the second one with a written config, through `TestEnv`) and compare the
    printed JSON field by field as today.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test reports` and
  `cargo test -p tdo --test cli`. Expected: unknown method `export`.
- [ ] **Step 3: Implement.** Implement `disk_snapshot`, and the handler that builds the
  document from it on the request thread. The client prints
  `skipped <n> unparseable resources` on stderr when `skipped > 0`, then the document with
  `to_string_pretty`.
- [ ] **Step 4: Regenerate the schema and run `cargo test -p twydo-daemon -p tdo`.** All green.
- [ ] **Step 5: Commit.** `feat(daemon): export; tdo export-json asks the daemon`

### Task 3: `conflicts.show`; `tdo conflicts` and `tdo conflicts show` through the daemon

**Files:**
- Modify: `crates/twydo-daemon/src/protocol/admin.rs`, `server/admin.rs`, `dispatch.rs`,
  `schema.rs`, `schema/tdo.openrpc.json`
- Modify: `crates/tdo/src/cli/conflicts.rs` (`list` and `show` become clients; `hook` is unchanged)
- Test: `crates/twydo-daemon/tests/admin.rs`, `crates/tdo/tests/sync_e2e.rs` (the existing
  `conflicts` CLI tests)

**Interfaces:**
- Produces:
  - `ConflictsShowParams { id: String }`
  - `VersionView { summary: Option<String>, due: Option<String>, status: Option<String>, last_modified: Option<String>, file: Option<PathBuf> }`.
    `file` is absolute. It is `None` for the current version, whose path is `vdir_file`.
  - `ConflictDetail { conflict: ConflictView, local: Option<VersionView>, remote: Option<VersionView>, current: Option<VersionView>, vdir_file: PathBuf }`.
    `None` means deleted for local and remote, and absent for current.
  - method `conflicts.show`. An unknown id is refused `not_found` with `no conflict <id>`.
  - `Conflict::field(text: &str, name: &str) -> Option<String>` in `twydo-core::conflicts`,
    replacing both copies of the SUMMARY extraction (a deferred plan-2 item).

- [ ] **Step 1: Write the failing tests.**
  - `admin.rs::conflicts_show_reports_both_sides_and_current`: a recorded conflict, with remote
    deleted and the current file present, returns `local.summary`, `remote: None`,
    `current.status` and the absolute `local.file` under `<state>/conflicts/<id>/local.ics`.
  - `conflicts_show_unknown_id_is_not_found`.
  - The CLI tests that cover `tdo conflicts` and `tdo conflicts show` keep their expected
    stdout byte for byte (`local:`, `  SUMMARY: …`, `  file: …`, `current:`, `vdir file: …`).
    Add `conflicts_show_unknown_id_exits_2` (stderr `tdo: no conflict nope`).
- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-daemon --test admin`.
  Expected: unknown method `conflicts.show`.
- [ ] **Step 3: Implement.** Implement the handler (reads the conflict store and the vdir file on
  the request thread). Then the clients: `list` calls `conflicts.list` and prints today's line
  (`chosen: keep local` and so on from `state` and `choice`, and `no conflicts` when empty).
  `show` calls `conflicts.show` and prints today's layout.
- [ ] **Step 4: Regenerate the schema and run `cargo test -p twydo-daemon -p tdo`.** All green.
- [ ] **Step 5: Commit.** `feat(daemon): conflicts.show; tdo conflicts reads through the daemon`

### Task 4: `tdo status` through the daemon; no client reads files

**Files:**
- Modify: `crates/tdo/src/cli/sync.rs` (`status`), `crates/tdo/src/main.rs`
- Modify: `crates/tdo/src/cli/mod.rs`: delete `load_vdir`, `reader_lock`,
  `permission_snapshot`, `recover_at_startup` and `CliError::{Vdir, Lock}`. `resolve_config`
  stays, used by `tdo daemon` and `tdo conflicts hook`.
- Create: `crates/tdo/tests/no_direct_reads.rs`
- Test: `crates/tdo/tests/cli.rs` (status tests), `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Consumes: `conflicts.list` (`ConflictList`), `status.get` (`StatusView`) and `snapshot.get`
  (`WireSnapshot`), all existing.
- Produces: `cli::sync::status(target: &Target) -> i32`.

- [ ] **Step 1: Write the failing tests.**
  - The existing status tests in `cli.rs` (`status_prints_never_synced`,
    `clean_sync_exits_0_and_status_shows_it`, `failed_sync_exits_1` and the rest) keep their
    expected lines. Add `status_without_sync_exits_2`: no `[sync]` gives
    `tdo: <NOT_CONFIGURED>` and exit 2.
  - `cli_op.rs::read_only_commands_never_open_the_config_file`: like the existing `op` test at
    line 1090. Start the daemon, make `config.toml` mode 000, then run `check`, `export-json`,
    `status`, `conflicts` and `op check <uid>`. All succeed.
  - `no_direct_reads.rs::clients_do_not_read_the_vdir_or_config`: scans `crates/tdo/src/**/*.rs`
    for `vdir::load`, `existing_vdir_state`, `ConflictStore`, `Config::load`, `resolve_config`,
    `store::journal` and `fs::read`. It allows only `cli/daemon.rs`, `cli/mod.rs`
    (the definition of `resolve_config`), the `hook` function's file `cli/conflicts.rs` for
    `resolve_config` only, `editor.rs` (the notes temp file) and `#[cfg(test)]` blocks. The
    failure message names the file and line.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test no_direct_reads --test cli_op`.
  Expected: hits in `cli/sync.rs` and `cli/mod.rs`.
- [ ] **Step 3: Implement `status`.** Print `warning: <w>` for each `config_warnings` entry, then
  today's lines. Times use the snapshot configuration's timezone. `pending:` prints
  `journal.unfinished`, conflicts come from `conflicts.list` (`id path kind`), and the `lists:`
  section is built from `sync.collections` through the same words as `describe` (add
  `fn describe_wire(&WireCollState) -> String` next to the client, and print `unknown` for
  `Unknown`). Delete the helpers listed under Files.
- [ ] **Step 4: Run `cargo test -p tdo` and `cargo clippy --all-targets -- -D warnings`.** All green.
- [ ] **Step 5: Commit.** `feat(tdo): tdo status asks the daemon; no client reads the vdir`

### Task 5: the service module: unit names, rendering, the stable path, the manager seam

**Files:**
- Create: `crates/tdo/src/service/mod.rs` (`Unit`, `unit_for`, `stable_exe`, `Manager`),
  `crates/tdo/src/service/systemd.rs` and `crates/tdo/src/service/launchd.rs` (rendering and
  commands)
- Modify: `crates/tdo/src/lib.rs`, `crates/twydo-daemon/src/paths.rs` (`default_socket`)
- Replace: `contrib/systemd/twydo-sync.{service,timer}` with `contrib/systemd/twydo.service`,
  and `contrib/launchd/app.twydo.sync.plist` with `contrib/launchd/app.twydo.daemon.plist`.
  Both are reference copies.
- Test: unit tests in `service/*.rs`, `crates/tdo/tests/service_units.rs`

**Interfaces:**
- Produces:
  - `paths::default_socket() -> PathBuf`: `socket_path_from(None, …)` with `TWYDO_SOCKET` ignored.
  - `enum Platform { Systemd, Launchd }` and `Platform::current()` (macOS → Launchd, else Systemd).
  - `struct Unit { platform: Platform, name: String /* twydo.service | app.twydo.daemon[.<id>] */, file: PathBuf, socket: PathBuf, default_socket: bool }`
  - `fn unit_for(platform: Platform, socket: &Path, env: &dyn Fn(&str) -> Option<OsString>) -> Unit`
    (paths from `XDG_CONFIG_HOME`/`HOME` through `env`) and `fn unit_for_current(socket: &Path) -> Unit`.
  - `struct Spec { exe: PathBuf, socket: PathBuf, default_socket: bool, vdir: Option<PathBuf>, env: Vec<(String, String)>, log: PathBuf }`
  - `fn render(unit: &Unit, spec: &Spec) -> String` (systemd unit text or plist XML; the
    plist escapes `&`, `<` and `>`).
  - `fn stable_exe(path_var: Option<&OsStr>, mise_shims: &Path) -> Result<PathBuf, String>`
    and `fn service_env(env: &dyn Fn(&str) -> Option<OsString>) -> Vec<(String, String)>`
    (`PATH` plus the set `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_DATA_HOME`, in that order).
  - `trait Manager { fn reload(&self) -> io::Result<()>; fn enable_now(&self, u: &Unit) -> io::Result<()>; fn start(&self, u: &Unit) -> io::Result<()>; fn stop(&self, u: &Unit) -> io::Result<()>; fn disable_now(&self, u: &Unit) -> io::Result<()>; fn state(&self, u: &Unit) -> UnitState; fn disable_old_timer(&self) -> io::Result<Option<String>>; }`
    `UnitState { enabled: Option<bool>, active: Option<bool> }` (`None` = the manager did not say).
    `fn manager(p: Platform) -> Box<dyn Manager>` runs `systemctl --user …`
    (`daemon-reload`, `enable --now`, `start`, `stop`, `disable --now`, `is-enabled`,
    `is-active`) or `launchctl …` (`bootstrap gui/<uid> <plist>`, `kickstart`, `bootout`,
    `print`). A non-zero exit becomes `io::Error::other("<cmd>: <stderr>")`.
    `disable_old_timer` returns the name it disabled.

- [ ] **Step 1: Write the failing tests.**
  - `systemd_unit_for_the_default_socket`: `render` for `Spec { exe: /home/u/.local/share/mise/shims/tdo, default_socket: true, vdir: None, env: [PATH=/usr/bin, XDG_CONFIG_HOME=/home/u/cfg] }`
    contains, each on its own line: `ExecStart=/home/u/.local/share/mise/shims/tdo daemon`,
    `Environment="PATH=/usr/bin"`, `Environment="XDG_CONFIG_HOME=/home/u/cfg"`,
    `Restart=always`, `RestartSec=1`, `RestartPreventExitStatus=10`, `TimeoutStopSec=90`,
    `WantedBy=default.target`. It does not contain `--socket`.
  - `systemd_unit_for_another_socket_names_it`: the unit is named `twydo-<socket_id>.service`,
    and `ExecStart` ends `daemon --socket /run/t/s.sock --vdir /v`.
  - `launchd_plist_always_names_the_socket`: label `app.twydo.daemon`; `ProgramArguments`
    contains `--socket` and the path; `RunAtLoad`, `KeepAlive`, `ExitTimeOut` 90,
    `StandardErrorPath` = the log; `EnvironmentVariables` holds `PATH`; a path with `&` is
    written as `&amp;`.
  - `unit_file_follows_xdg_config_home`: with `XDG_CONFIG_HOME=/c` the systemd file is
    `/c/systemd/user/twydo.service`; with only `HOME=/h` it is
    `/h/.config/systemd/user/twydo.service`; launchd uses `/h/Library/LaunchAgents/app.twydo.daemon.plist`.
  - `stable_exe_skips_versioned_installs`: with `PATH=/m/installs/twydo/0.3.0:/usr/local/bin`,
    where both hold `tdo` (tempdir files), the result is `/usr/local/bin/tdo`. With only the
    versioned entry and an existing `<shims>/tdo`, the result is the shim. With neither, the
    error is `no stable path to tdo (found only /m/installs/twydo/0.3.0/tdo); run 'mise reshim' or pass --exe <path>`.
  - `service_env_keeps_path_and_set_xdg_vars`.
  - `tests/service_units.rs::contrib_units_match_what_install_writes`: rendering with
    `exe = /home/you/.local/share/mise/shims/tdo`, the default socket, no vdir and
    `env = [PATH=/home/you/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin]` (on launchd:
    socket `/Users/you/Library/Caches/twydo/tdo.sock`, log
    `/Users/you/.local/state/twydo/daemon.log`) equals the contrib files byte for byte.
  - `manager_runs_systemctl_with_user`: with a fake `systemctl` first on `PATH` that logs
    `$@`, `enable_now` logs `--user enable --now twydo.service`. A fake that exits 1 with
    stderr `boom` gives an error containing `boom`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo service`. Expected: the
  module does not exist.
- [ ] **Step 3: Implement** the module, `paths::default_socket`, and the contrib files (each
  with a header comment: "reference copy; `tdo daemon install` writes this for you"). Delete
  the old contrib timer files.
- [ ] **Step 4: Run `cargo test -p tdo service` and `cargo test -p twydo-daemon paths`.** All green.
- [ ] **Step 5: Commit.** `feat(tdo): service unit naming, rendering and manager commands`

### Task 6: `tdo daemon install`, `uninstall`, `status` and `schema`

**Files:**
- Modify: `crates/tdo/src/cli/daemon.rs` (new `DaemonCommand` variants; `stop` logic factored
  into `fn stop_detached(target) -> Result<bool, String>`), `crates/tdo/src/main.rs`
  (`tdo daemon` is no longer hidden)
- Create: `crates/tdo/src/cli/service.rs` (install, uninstall, status)
- Test: `crates/tdo/tests/daemon_install.rs`, `crates/tdo/tests/common/mod.rs`
  (`fake_service_manager(dir) -> (PathBuf /* bin dir */, PathBuf /* log */)`)

**Interfaces:**
- Consumes: Task 5's `unit_for_current`, `render`, `stable_exe`, `service_env`, `manager`;
  `client::start::log_path`; `client::wait_gone`.
- Produces:
  - `DaemonCommand::Install { #[arg(long)] exe: Option<PathBuf> }`, `Uninstall`, `Status`, `Schema`
  - `cli::service::install(target: &Target, exe: Option<&Path>, mgr: &dyn Manager) -> i32`,
    `uninstall(target, mgr) -> i32`, `status(target, mgr) -> i32`
  - `client::probe(t: &Target) -> Probe` where `enum Probe { Running(DaemonInfo), WrongInstance(String), NotRunning }`.
    It says hello without starting anything. Task 8 reuses it.

- [ ] **Step 1: Write the failing tests** (every test uses `TestEnv` plus `HOME`, the fake
  manager and a `PATH` of `<fake bin>:<dir holding a tdo symlink to CARGO_BIN_EXE_tdo>:/usr/bin:/bin`):
  - `install_writes_the_unit_and_enables_it`: exit 0. The unit file under
    `$XDG_CONFIG_HOME/systemd/user/twydo-<id>.service` names the symlinked `tdo`, `--socket`
    and `XDG_CONFIG_HOME`. The manager log shows `--user daemon-reload` then
    `--user enable --now twydo-<id>.service`. stdout says `installed twydo-<id>.service`.
  - `install_is_idempotent`: a second run rewrites the same bytes and says
    `twydo-<id>.service was already installed; rewrote it`.
  - `install_stops_a_detached_daemon_first`: with a detached daemon answering, the run ends
    with that daemon gone (its socket stops answering, or answers with another epoch) before
    `enable --now` appears in the log.
  - `install_disables_the_old_timer`: with `systemd/user/twydo-sync.timer` present, the log
    shows `--user disable --now twydo-sync.timer`, stdout says
    `disabled twydo-sync.timer: the daemon syncs now`, and the file is still there.
  - `install_refuses_a_versioned_exe`: a `PATH` holding only `…/installs/twydo/0.1.0/tdo`
    exits 2 with the Task-5 message, and writes no file.
  - `uninstall_disables_and_removes`: the log shows `--user disable --now <unit>`, the file is
    gone, and `daemon-reload` runs. Without a unit it prints `no unit installed` and exits 0.
  - `status_reports_unit_and_daemon_without_starting_one`: no unit and no daemon gives
    `unit: not installed; daemon won't start at login; run tdo daemon install` and
    `daemon: not running`, and **no daemon is started** (the socket still does not exist
    afterwards). With a unit and a running daemon, it prints `unit: installed (<path>), enabled`
    (the fake `is-enabled` prints `enabled`) and
    `daemon: running, version <v>, pid <p>, vdir <path>`.
  - `schema_prints_the_document`: `tdo daemon schema` output parses as JSON and equals
    `schema/tdo.openrpc.json`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test daemon_install`.
  Expected: unknown subcommands.
- [ ] **Step 3: Implement.**
  - `install`: resolve the exe, then render. Stop a detached daemon of this target (its own
    instance only; a `WrongInstance` probe refuses with its message), write the file mode
    0644, `reload`, `enable_now`, `disable_old_timer`. Finally wait up to `START_WAIT` for
    `probe` to return `Running`. If it does not, exit 2 with
    `the service did not start; see journalctl --user -u <unit>` (on launchd: `see <log>`).
  - `uninstall`: `disable_now`, remove the file, `reload`.
  - `status`: report the unit and the manager state, then `probe`.
- [ ] **Step 4: Run `cargo test -p tdo`.** All green.
- [ ] **Step 5: Commit.** `feat(tdo): tdo daemon install, uninstall, status and schema`

### Task 7: start, stop and upgrade through the unit

**Files:**
- Modify: `crates/tdo/src/client/start.rs` (`start(t)` chooses the manager or the detached start),
  `crates/tdo/src/client/mod.rs` (`connect`, `restart_on` and `wait_for_daemon` call
  `start::start`; `wait_replaced`), `crates/tdo/src/cli/daemon.rs` (`stop` through the
  manager when a unit is installed)
- Modify: `crates/twydo-daemon/src/server/dispatch.rs` (`when_idle` parsed once; deferred
  plan-2 item T6)
- Test: `crates/tdo/tests/daemon_install.rs`, `crates/tdo/tests/daemon_process.rs`,
  `crates/twydo-daemon/tests/lifecycle.rs`

**Interfaces:**
- Consumes: Task 5's `Manager`, `unit_for_current`; Task 6's fake manager helper.
- Produces:
  - `client::start::start(t: &Target) -> io::Result<()>`: with a unit file present it calls
    `manager.start`, else `detached`. If the manager call fails it starts detached and returns
    `Ok`, and the manager's error is appended to the final "no daemon answered" message.
  - `client::wait_replaced(socket: &Path, old_epoch: Option<&str>, limit: Duration) -> bool`:
    true once the socket is gone (as `wait_gone`) or a daemon answers `daemon.info` with an
    epoch other than `old_epoch`.

- [ ] **Step 1: Write the failing tests.**
  - `daemon_install.rs::a_client_starts_the_daemon_through_the_unit`: with a unit file present
    and a fake `systemctl start` that runs `tdo daemon --detach --socket <s>` (simulating the
    manager), `tdo status` succeeds and the log shows `--user start twydo-<id>.service`.
  - `a_failing_manager_falls_back_to_the_detached_start`: the fake `start` exits 1, and the
    client still connects through the detached start.
  - `stop_goes_through_the_manager_when_a_unit_is_installed`: the fake `stop` logs its
    arguments and sends the daemon SIGTERM (pid from `daemon.info`). `tdo daemon stop` exits 0,
    the log shows `--user stop twydo-<id>.service`, and the socket stops answering.
  - `daemon_process.rs::upgrade_accepts_a_daemon_restarted_by_the_manager`: an "older" daemon
    (the existing version-override hook of plan 3) is replaced by a newer client. A watcher
    thread starts a new daemon on the same socket as soon as the old one exits. The client
    connects within 5 s and does not report "still busy".
  - `lifecycle.rs::shutdown_when_idle_races_job_creation`: 50 rounds of `sync.run` sent
    concurrently with `daemon.shutdown {when_idle: true}`. Every round ends with the job either
    refused `unavailable` or recorded `interrupted` or finished. None stays `queued`.
- [ ] **Step 2: Run them and check they fail.** `cargo test -p tdo --test daemon_install --test daemon_process`
  and `cargo test -p twydo-daemon --test lifecycle`.
- [ ] **Step 3: Implement.** Implement `start::start` and `wait_replaced`. `restart_on` waits with
  `wait_replaced(socket, seen_epoch_or_info_epoch, STOP_WAIT)` and does not start anything when
  a new epoch already answers. `stop` checks `unit_for_current(&target.socket).file.exists()`
  first. `is_shutdown` returns the parsed `ShutdownParams` so dispatch does not parse twice.
- [ ] **Step 4: Run `cargo test`.** All green.
- [ ] **Step 5: Commit.** `feat(tdo): start, stop and upgrade the daemon through its service unit`

### Task 8: the warning while no unit is installed

**Files:**
- Modify: `crates/twydo-core/src/config.rs` (`[daemon]` table), `crates/twydo-daemon/src/protocol/wire.rs`
  (`unit_warning`), `crates/tdo/src/term.rs` (`unit_notice`), `crates/tdo/src/main.rs`,
  `schema/tdo.openrpc.json`
- Test: `crates/twydo-core/tests/config.rs` (or the config unit tests), `crates/twydo-daemon/tests/wire.rs`,
  `crates/tdo/tests/tui.rs`

**Interfaces:**
- Produces:
  - `Config.daemon: DaemonPrefs` with `#[serde(default, skip_serializing)]`, and
    `DaemonPrefs { unit_warning: bool }` (`deny_unknown_fields`, default `true`).
  - `WireSnapshot.unit_warning: bool`, `#[serde(default = "yes")]`, set from
    `snap.config.daemon.unit_warning` in `to_wire`. `from_wire` copies it back into
    `Config.daemon`.
  - `term::unit_notice(app: &mut App, unit_installed: bool)`: calls
    `app.notice("daemon won't start at login; run tdo daemon install")` when
    `!unit_installed && app.snapshot.config.daemon.unit_warning`. `main.rs` calls it once after
    `first_app`, with `service::unit_for_current(&target.socket).file.exists()`.

- [ ] **Step 1: Write the failing tests.**
  - Config: `[daemon]\nunit_warning = false` parses; `[daemon]\nfoo = 1` is an error; the
    default is `true`.
  - `wire.rs::wire_config_never_carries_the_daemon_table`:
    `serde_json::to_value(&to_wire(..).config)` has no `daemon` key, and `unit_warning` is
    `false` when the file says so.
  - `wire.rs::a_snapshot_without_unit_warning_decodes_as_true`: the JSON of a plan-3
    `WireSnapshot` (built with the key removed) decodes with `unit_warning == true`.
  - `tui.rs::warns_once_without_a_unit`: `unit_notice(&mut app, false)` sets the message once.
    With `true`, or with `unit_warning = false` in the config, no message is set.
  - `tui.rs::the_warning_switch_reaches_the_tui_live`: changing `unit_warning` in
    `config.toml` reaches the TUI's snapshot without a restart (`cause: config`).
- [ ] **Step 2: Run them and check they fail.** `cargo test -p twydo-core config`,
  `cargo test -p twydo-daemon --test wire` and `cargo test -p tdo --test tui`.
- [ ] **Step 3: Implement**, regenerate the schema, and run the breaking-change test. A new
  result field is an addition, so it passes.
- [ ] **Step 4: Run `cargo test`.** All green.
- [ ] **Step 5: Commit.** `feat(tdo): warn while no service unit is installed`

### Task 9: documentation

**Files:**
- Modify: `docs/install.md` (service setup with `tdo daemon install`, upgrades, what the unit
  does, `tdo daemon status`/`stop`/`uninstall`, macOS notes), `docs/sync.md` (the daemon
  syncs; the timer is retired; do not run `vdirsyncer sync` by hand while the daemon runs),
  `docs/config.md` (who reads the file; `[daemon] unit_warning`; the 60 s interval is fixed;
  `check --matrix`), `docs/architecture.md` (nothing reads files; the read-only methods;
  service start), `docs/tui.md` (the unit warning, `daemon restarted`),
  `docs/protocol.md` (the new methods, `tdo daemon schema`), `docs/compatibility.md` (the
  matrix belongs to the daemon; `check --matrix`)
- Modify: `docs/superpowers/specs/2026-11-09-daemon-design.md` (§6.2: `tdo daemon schema`
  built; §10.2: the macOS `KeepAlive` caveat and the stable-path rule, marked as builder
  decisions of plan 4)
- Modify: `AGENTS.md` (progress note: daemon plan 4 done; next: write-queue plan 3, then Plan 2)

- [ ] **Step 1: Write the pages.** Remove every "still reads the file" statement (see
  `docs/architecture.md:67`, `:385`, `docs/config.md:9`, `:52`). Every command named exists in
  `tdo --help` / `tdo daemon --help`.
- [ ] **Step 2: Check.** Run `python3 tools/check_links.py` and
  `grep -rn 'twydo-sync.timer\|still read' docs README.md`. The links pass, and the grep finds
  the timer only where `install` is said to disable it.
- [ ] **Step 3: Commit.** `docs: read-only commands, the service unit and setup`

---

## Deferred review items: what this plan does with each

| Item | Decision |
|---|---|
| Plan 3: service-unit stop timeout (`TimeoutStopSec`) | Task 5: `TimeoutStopSec=90`, launchd `ExitTimeOut` 90 |
| Plan 2 T6: `when_idle` parsed twice; no race test against job creation | Task 7 |
| Plan 2 T7: SUMMARY extraction duplicated | Task 3 (`Conflict::field`) |
| Plan 2 T6: run-lock shutdown test ~10 s; duplicated `write_script`/`dead_pid` | Later: test hygiene |
| Plan 2 T10: stale fence defers unrelated edits | Later: needs its own design |
| Plan 3: watch retry test assumes non-root; downgrade drops `Conflict.earlier` | Later |

## After the plan

Ask the user before running `tdo daemon install` for the production setup, and separately for
the phase-0 test setup (its own socket, so its own unit). The old `twydo-sync.timer` of the
test setup is already disabled; install reports it anyway.
