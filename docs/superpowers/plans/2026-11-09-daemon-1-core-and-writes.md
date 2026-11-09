# Daemon plan 1: core, protocol and task writes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `tdo daemon` that owns one vdir and serves snapshots, status, events and task writes
(`op.submit`, `op.undo`, `op.redo`, `op.get`, `refusals.ack`) over a Unix socket, with sessions,
`request_id` replay, `based_on` planning, and the write queue inside the daemon. No client uses it yet.

**Architecture:** A new library crate `twydo-daemon` holds the protocol types (JSON-RPC 2.0, one
message per line), the generated OpenRPC schema, the server and a small blocking client. The server
is plain threads: one per connection for reading, one per connection for writing, one per request.
A **lane token** gives the vdir to one holder at a time: a short write, or a long job (a sync or a
list job, built in plan 2). While a long job holds it, writes are gated, planned and queued
without waiting. Each client session owns a `twydo_core::store::Store`, so the store's existing undo
and redo stacks become per-session (D7) without changing them.

**Tech Stack:** Rust 2024 (workspace `rust-version = "1.99"`), serde/serde_json, `schemars` 1.x
(with `chrono04`), `notify` 8 (already used by `tdo`), `libc` (peer credentials, uid),
`signal-hook` 0.3 (in `tdo` only), `tempfile` for tests.

**Spec:** [`docs/superpowers/specs/2026-11-09-daemon-design.md`](../specs/2026-11-09-daemon-design.md)
(read §5–§7 and §13 before starting; the main spec §11 contracts bind everything).

## Where this plan sits

The spec's rollout step 1 (§12) is too big for one plan. It is split into three plans on the branch
`feat/twydo-daemon`. The **clients switch to the daemon only in plan 3**, so there is never a time
when the daemon and a client both write as owners:

| Plan | Delivers | Clients |
|---|---|---|
| **1 (this)** | crate, protocol, schema, lifecycle, snapshot/status/events, watcher, task writes with sessions, replay and `based_on`, queue and drain, `refusals.ack`, hidden `tdo daemon` | unchanged |
| 2 | sync jobs and the scheduler (startup, periodic, batched, after-resolve, wake), `sync.*`, orphaned vdirsyncer at startup, `job.get`, `conflicts.*`, `journal.*`, `lists.*` jobs, `input.parse`, `config.toml` reload (cause `config`, restart-needed settings in status), shutdown during a sync | unchanged |
| 3 | client interface, on-demand start, upgrade rule, TUI and every mutating CLI command over the socket, the TUI's configuration from the snapshot (`PreferenceWatch` and `resolve_config` leave the clients); `Writer`, waiter and `spawn*` removed; docs | switched |

Rollout steps 2–4 of the spec follow as their own plans.

## Decisions this plan makes (builder's calls in the spec)

- **Threads, no async runtime.** The codebase is synchronous (`std::thread`, `mpsc`); vdirsyncer
  runs are blocking. Each request runs on its own short-lived thread, so a slow write never delays a
  `snapshot.get` on the same connection (anti-goal: the UI waits on the socket).
- **New crate `crates/twydo-daemon`.** `twydo-core` stays free of sockets; `tdo` depends on the new
  crate for the `daemon` subcommand now and for the client in plan 3.
- **Per-session `Store`.** The undo rules of today (write-queue design §6.7) stay in one place.
- **Wire tasks reuse `model::Task`** (serde + schemars derives added), with `path` sent
  **vdir-relative** (`<collection>/<file>`), not dropped as §6.4 suggests: the TUI uses it to name
  tasks in messages and the errors pane, and it holds nothing private. Raw bytes are not sent; each
  task carries `rev` (first 16 hex digits of the SHA-256 of its file) and `pending`.
- **The snapshot carries the whole loaded `Config` and its warnings** (spec D13). It is the
  user's own file on a same-user socket, so nothing is filtered; clients have no other copy.
- **Success messages come from the daemon.** `WriteResult.message` carries today's TUI wording
  (`"saved"`, `"done: \"X\" and 2 items"`, `"undone: X"`), so every client prints the same text.
  `op_title` and the success wording move from `tdo/src/write.rs` into `twydo_core::edit::describe`.
- **Refusal acknowledgement reuses the refused-items store:** "unacknowledged" is today's
  `shown == false`, and `refusals.ack` calls `Queue::mark_shown`. No file format change.
- **Error kind `internal`** is added to the spec's table for I/O failures.
- **Kept snapshots:** the last 32 published revisions.
- **Exit code 10** for "another daemon serves this vdir" (`EXIT_ALREADY_RUNNING`).
- **Socket path** (spec D13, §6.1): `--socket`, else `TWYDO_SOCKET`, else the default
  `$XDG_RUNTIME_DIR/twydo/tdo.sock` on Linux, `$TMPDIR/twydo/tdo.sock` on macOS, and
  `/tmp/twydo-<uid>/tdo.sock` without either variable. No config key: clients do not read the
  configuration.
- **Instance identity** (checked by `hello`, never used to pick the socket):
  `Instance { config: PathBuf, vdir: Option<PathBuf> }` = `Config::path()` and the `--vdir` flag,
  both made absolute (`std::path::absolute`; no file is read or required to exist).
- **Message cap:** 16 MiB per line.

## Global Constraints

- Contracts in the main spec §11 (compatibility gate, preservation, conflict, recovery,
  notification) bind everything; "nothing is ever planned against a newer snapshot than the client
  saw" (daemon spec §6.6).
- On-disk data unchanged except **optional** fields: journal entries and queue items gain
  `request` (`{session, request_id}`), `serde(default, skip_serializing_if = "Option::is_none")`;
  older files stay readable (spec goal 7).
- Pure modules (`ical`, `model`, `views`, `dateinput`, `repeatinput`, `edit`, `gate`) stay free of
  file, process and clock access; derives are fine, I/O is not.
- Every test that starts a daemon or runs `tdo` sets its own socket (`DaemonOptions::socket` or
  `TWYDO_SOCKET`): no test may reach or start a daemon at the user's default socket.
- Only the daemon reads `config.toml` (spec D13). Nothing in `twydo-daemon`'s client side may call
  `Config::load` or `resolve_config`.
- JSON-RPC 2.0, UTF-8, one message per `\n`-terminated line, both directions; responses carry the
  request `id`; notifications (no `id`) are events.
- `PROTOCOL: u32 = 1`. Lifecycle methods `hello`, `rpc.discover`, `daemon.info`, `daemon.shutdown`
  answer without a successful `hello`; everything else needs one.
- Error kinds (`data.kind`): `blocked`, `stale`, `stuck`, `not_found`, `invalid`, `unavailable`,
  `protocol`, `wrong_instance`, `internal`. Message wording follows today's messages.
- Socket directory `0700`, socket `0600`, peer uid must equal the daemon's uid.
- No personal data in the repository: every fixture is synthetic (`AGENTS.md`).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass at every commit. Conventional commit messages (release-plz), no
  agent attribution lines.

## Review Focus

1. **A resend while the first request is still running** (two connections, same `request_id`,
   e.g. a client that timed out and reconnected) must apply once and give both callers the same
   result. Test: `concurrent_resend_applies_once` (Task 7).
2. **The daemon's own write seen by its watcher** must not publish a second revision or a second
   `data.changed`. Test: `own_write_publishes_one_revision` (Task 9).
3. **A subscriber that never reads** must get `missed`, never block writes or other subscribers.
   Test: `stalled_subscriber_does_not_block_others` (Task 5).
4. **Undo after a queued edit was refused** must skip that edit and name the right task, as the
   TUI does today. Test: `undo_skips_a_refused_queued_edit` (Task 8).
5. **Garbage on the socket** (invalid JSON, a line over the cap, a request before `hello`) gets a
   JSON-RPC error or a closed connection, and the daemon keeps serving others.
   Test: `malformed_input_does_not_hurt_other_clients` (Task 4).

---

## File structure

```
crates/twydo-daemon/
  Cargo.toml
  src/lib.rs               re-exports; PROTOCOL; EXIT_ALREADY_RUNNING
  src/protocol/mod.rs      module root
  src/protocol/rpc.rs      JSON-RPC envelope, RpcError, ErrorKind, ErrorData, line codec
  src/protocol/methods.rs  params/results of every plan-1 method
  src/protocol/events.rs   EventEnvelope, Event, Cause, Missed, Heartbeat
  src/protocol/wire.rs     WireSnapshot, WireTask, WireCollection, to_wire, from_wire, rev_of
  src/schema.rs            OpenRPC document builder + breaking-change checker
  src/paths.rs             socket_path, DaemonLock
  src/client.rs            blocking Client
  src/server/mod.rs        Daemon::start, DaemonHandle, DaemonOptions, accept loop, dispatch
  src/server/conn.rs       per-connection reader/writer threads, peer check
  src/server/publish.rs    Publisher: revisions, kept snapshots, StatusView
  src/server/bus.rs        EventBus: seq, subscribers, missed, heartbeat
  src/server/lane.rs       Lane token: Free / Write / Long
  src/server/sessions.rs   Sessions: per-session Store and title stacks, expiry
  src/server/replay.rs     RequestLog: in-flight and done results, durable lookup
  src/server/writes.rs     op.submit / undo / redo / get / check, refusals.ack, error mapping
  src/server/watch.rs      vdir watcher (moved logic from tdo/src/term.rs)
  tests/common/mod.rs      TestDaemon, fixtures, helpers
  tests/lifecycle.rs  tests/read.rs  tests/events.rs  tests/writes.rs  tests/queue.rs  tests/schema.rs
schema/tdo.openrpc.json        generated, checked in
schema/released.openrpc.json   last released API (starts as a copy)
crates/twydo-core/src/edit/describe.rs   op_title, success_message (moved from tdo)
crates/tdo/src/cli/daemon.rs             hidden `tdo daemon` subcommand
crates/tdo/tests/daemon_process.rs       kill -9 and signal tests against the binary
```

Modified in `twydo-core`: `Cargo.toml` (schemars), `config.rs` (Serialize + JsonSchema), `model/task.rs`, `model/meta.rs`,
`ical/value.rs`, `edit/mod.rs`, `repeatinput.rs`, `vdir.rs` (derives); `store/journal.rs`,
`store/queue.rs`, `store/mod.rs` (request records, hooks); `sync/mod.rs` (`pub mod lock`);
`views.rs` (`rev`). Modified in `tdo`: `Cargo.toml`, `main.rs`, `write.rs` (use `describe`).

---

### Task 1: Wire types for tasks and the snapshot

**Files:**
- Modify: `Cargo.toml` (workspace members), `crates/twydo-core/Cargo.toml`
- Modify: `crates/twydo-core/src/config.rs`, `model/task.rs`, `model/meta.rs`, `ical/value.rs`, `edit/mod.rs`,
  `repeatinput.rs`, `vdir.rs`, `views.rs`
- Create: `crates/twydo-daemon/Cargo.toml`, `src/lib.rs`, `src/protocol/mod.rs`, `src/protocol/wire.rs`
- Test: `crates/twydo-daemon/tests/wire.rs`

**Interfaces:**
- Produces:
  - `twydo_daemon::protocol::wire::{WireSnapshot, WireTask, WireCollection}`:
    `WireSnapshot { revision: u64, config: Config, config_warnings: Vec<String>, collections: Vec<WireCollection>, tasks: Vec<WireTask>, errors: Vec<WireError> }`,
    `WireTask { #[serde(flatten)] task: Task, rev: String, pending: bool }` (`task.path` vdir-relative),
    `WireCollection { id, display_name: Option<String>, color: Option<String /* #rrggbb */> }`,
    `WireError { path: String /* vdir-relative */, message: String }`.
  - `pub fn to_wire(snap: &Snapshot, revision: u64) -> WireSnapshot`
  - `pub fn from_wire(w: WireSnapshot) -> Snapshot` (the snapshot's config is `w.config`; paths
    re-joined with `w.config.vdir`; pending and revs restored; no sources). `to_wire` fills
    `config_warnings` from `Config::warnings()`.
  - `pub fn rev_of(bytes: &[u8]) -> String` (16 lowercase hex chars of SHA-256).
  - `Snapshot::rev(&self, idx: usize) -> Option<&str>` and `Snapshot::with_revs(self, HashMap<String, String>) -> Snapshot`
    (uid → rev). `Snapshot::new` fills revs from `sources`; `from_wire` uses `with_revs`.
  - `Serialize + Deserialize + JsonSchema` on `model::{Task, TaskKind, Status, Alarm, Trigger}`,
    `model::meta::{Metadata, MetaFormat}`, `ical::value::IcalDateTime`,
    `edit::{Op, NewTask, Change, Due, Placement}`, `repeatinput::RepeatSpec`; serde `rename_all = "snake_case"`
    **only on types that have no serde derive yet** (queue items already store `Op`: its encoding
    must not change). `Config` and its parts get `Serialize + JsonSchema` that **round-trip through
    its existing `Deserialize`** (same field names, defaults and renames). `Trigger::Relative.offset` (a `chrono::Duration`) serializes as whole seconds
    (`#[serde(with = ...)]`, `#[schemars(with = "i64")]`).

- [ ] **Step 1: Write the failing tests** in `crates/twydo-daemon/tests/wire.rs`:

```rust
#[test]
fn wire_round_trip_keeps_what_views_need() {
    let fx = common::Fixture::new(&[
        ("inbox", "p1", "Synthetic project", None),
        ("inbox", "c1", "Synthetic item one", Some("p1")),
        ("inbox", "c2", "Synthetic item two", Some("p1")),
    ]);
    let snap = fx.snapshot().with_pending(["c1".to_string()].into());
    let back = from_wire(serde_json::from_str(&serde_json::to_string(&to_wire(&snap, 7)).unwrap()).unwrap());
    assert_eq!(back.config, fx.config(), "the client's configuration comes from the snapshot");
    assert_eq!(back.tasks.len(), 3);
    let p = back.find("p1").unwrap();
    assert_eq!(back.children(p).len(), 2, "parent links resolve on the client");
    assert_eq!(back.tasks[p].kind, TaskKind::Project);
    assert!(back.is_pending(back.find("c1").unwrap()));
    assert_eq!(back.tasks[p].path, fx.vdir().join("inbox/p1.ics"));
    assert_eq!(back.rev(p), snap.rev(snap.find("p1").unwrap()));
}

#[test]
fn wire_paths_are_vdir_relative_and_carry_no_bytes() {
    let fx = common::Fixture::new(&[("inbox", "u1", "Synthetic", None)]);
    let json = serde_json::to_value(to_wire(&fx.snapshot(), 1)).unwrap();
    assert_eq!(json["tasks"][0]["path"], "inbox/u1.ics");
    assert!(!json.to_string().contains("BEGIN:VCALENDAR"));
    assert_eq!(json["tasks"][0]["rev"].as_str().unwrap().len(), 16);
}

#[test]
fn a_full_config_round_trips() {
    // every table set: lists prefs, groups, writes, sync, week_start, work_days, morning_time
    let cfg = common::full_config();
    let back: Config = serde_json::from_value(serde_json::to_value(&cfg).unwrap()).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn queued_op_encoding_is_unchanged() {
    // Queue files written before this change must still read.
    let op = Op::Edit { uid: "u1".into(), changes: vec![Change::Summary("X".into())] };
    assert_eq!(serde_json::to_string(&op).unwrap(), r#"{"Edit":{"uid":"u1","changes":[{"Summary":"X"}]}}"#);
}
```

`common::Fixture` (create `tests/common/mod.rs` now): a tempdir with `vdir/<collection>/<uid>.ics`
files built from synthetic VTODOs (`RELATED-TO;RELTYPE=PARENT:<parent>` when a parent is given),
`config()` = `Config::with_vdir(vdir)` with `state_dir` inside the tempdir and
`writes.mode = WriteMode::Test`, `snapshot()` = `Snapshot::new(vdir::load(..), config())`,
`write(rel, text)` and `remove(rel)` for outside changes.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p twydo-daemon --test wire`
Expected: compile errors (`to_wire`, `from_wire`, `Snapshot::rev` missing).

- [ ] **Step 3: Implement.** Add `schemars = { version = "1", features = ["chrono04"] }` to
`twydo-core` and `twydo-daemon`; `twydo-daemon` also depends on `twydo-core`, `serde`,
`serde_json`, `sha2`, `chrono`, `thiserror`, `uuid`. Derives as listed; `to_wire`/`from_wire` as
specified. `rev_of` lives in `twydo-core` (`views::rev_of`) so `Snapshot::new` can use it, and is
re-exported from `wire`.

- [ ] **Step 4: Run the tests.** `cargo test -p twydo-daemon --test wire` → PASS;
`cargo test --workspace` → PASS (existing queue tests prove the old encoding still reads).

- [ ] **Step 5: Commit** `feat(daemon): add wire types for tasks and snapshots`

---

### Task 2: JSON-RPC envelope, method types, events and line codec

**Files:**
- Create: `crates/twydo-daemon/src/protocol/rpc.rs`, `methods.rs`, `events.rs`
- Test: `crates/twydo-daemon/tests/rpc.rs`

**Interfaces:**
- Produces (all `Serialize + Deserialize + JsonSchema`, snake_case fields):
  - `rpc::Request { jsonrpc: String, id: Option<serde_json::Value>, method: String, params: Option<Value> }`,
    `rpc::Response` (result xor error), `rpc::RpcError { code: i64, message: String, data: Option<ErrorData> }`,
    `rpc::ErrorData { kind: ErrorKind, uid: Option<String>, current_rev: Option<String>, daemon: Option<String>, protocol: Option<u32> }`,
    `rpc::ErrorKind` (the nine kinds), `RpcError::refused(kind, message) -> RpcError` (code `-32000`),
    standard codes `PARSE_ERROR = -32700`, `INVALID_REQUEST = -32600`, `METHOD_NOT_FOUND = -32601`, `INVALID_PARAMS = -32602`.
  - `rpc::MAX_LINE: usize = 16 * 1024 * 1024`;
    `rpc::read_line(r: &mut impl BufRead) -> Result<Option<String>, LineError>` (`None` on EOF,
    `LineError::TooLong` without buffering past the cap, `LineError::Io`).
  - `methods.rs`: `Instance { config: PathBuf, vdir: Option<PathBuf> }` (Eq, Hash),
    `HelloParams { client, version, protocol: u32, session: Option<String>, instance: Instance }`,
    `HelloResult { daemon, protocol, epoch, session, vdir: PathBuf }`,
    `DaemonInfo { version, protocol, pid: u32, uptime_secs: u64, vdir: PathBuf, epoch: String }`,
    `ShutdownParams { when_idle: bool }`, `SubscribeResult { seq: u64, revision: u64, status_rev: u64 }`,
    `BasedOn { epoch: String, revision: u64 }`,
    `SubmitParams { session, request_id, based_on: BasedOn, op: Op }`,
    `UndoParams { session, request_id }` (also for redo),
    `WriteOutcome` = `applied | queued | satisfied`,
    `WriteResult { outcome, op_id: String, revision: u64, message: String, created: Option<Created { uid, rev }>, undid: Option<Undid { op_id: Option<String>, title: String }> }`,
    `OpGetParams { op_id: Option<String>, request_id: Option<String> }`,
    `OpState` = `pending | applied { op_id } | refused { kind: ErrorKind, message } | cancelled | satisfied { op_id }`
    (tag `state`), `CheckParams { based_on: BasedOn, op: Op }`, `CheckResult { allowed: bool, reason: Option<String> }`,
    `AckParams { request_id, ids: Vec<String> }`, `AckResult { acknowledged: usize }`,
    `StatusView { status_rev: u64, sync: Option<SyncView>, pending_edits: usize, conflicts: usize, journal: JournalView { unfinished: usize, stuck: Vec<String> }, refusals: Vec<RefusalView { id, message, session: Option<String>, request_id: Option<String>, at: DateTime<Utc> }> }`,
    `SyncView { state: SyncState /* idle | running */, auto: bool, last_success, last_attempt, next_attempt: Option<DateTime<Utc>>, failures: u32, last_error: Option<String> }`.
  - `events.rs`: `EventEnvelope { seq: u64, #[serde(flatten)] event: Event }`;
    `Event` tagged `type`: `"status.changed" { status_rev }`, `"data.changed" { revision, cause: Cause, uids: Vec<String> }`,
    `"op.finished" { op_id, request_id: Option<String>, state: OpState }`;
    `Cause` = `op | sync | outside | list | recovery | config`; `Missed { seq, revision, status_rev }`; `Heartbeat { seq }`.
    Notifications go out as methods `event`, `missed`, `heartbeat` (spec §6.5).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn refusal_carries_a_stable_kind() {
    let e = RpcError::refused(ErrorKind::Stale, "\"X\" changed on disk; reload and try again");
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["code"], -32000);
    assert_eq!(v["data"]["kind"], "stale");
}

#[test]
fn events_serialize_as_in_the_spec() {
    let e = EventEnvelope { seq: 120, event: Event::DataChanged { revision: 43, cause: Cause::Sync, uids: vec!["a".into()] } };
    assert_eq!(serde_json::to_value(&e).unwrap(),
        serde_json::json!({"seq":120,"type":"data.changed","revision":43,"cause":"sync","uids":["a"]}));
}

#[test]
fn read_line_refuses_an_oversized_line_without_buffering_it() {
    let big = vec![b'x'; MAX_LINE + 1];
    let mut r = std::io::BufReader::new(std::io::Cursor::new(big));
    assert!(matches!(read_line(&mut r), Err(LineError::TooLong)));
}

#[test]
fn read_line_splits_pipelined_messages() {
    let mut r = std::io::BufReader::new(&b"{\"a\":1}\n{\"b\":2}\n"[..]);
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("{\"a\":1}"));
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("{\"b\":2}"));
    assert_eq!(read_line(&mut r).unwrap(), None);
}
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test rpc` → FAIL (types missing).
- [ ] **Step 3: Implement** the types and `read_line` (read through `BufRead::fill_buf`, stop at the cap).
- [ ] **Step 4: Run** `cargo test -p twydo-daemon --test rpc` → PASS.
- [ ] **Step 5: Commit** `feat(daemon): define the JSON-RPC protocol types`

---

### Task 3: OpenRPC schema, staleness test and breaking-change check

**Files:**
- Create: `crates/twydo-daemon/src/schema.rs`, `schema/tdo.openrpc.json`, `schema/released.openrpc.json`
- Test: `crates/twydo-daemon/tests/schema.rs`

**Interfaces:**
- Consumes: Task 2 types.
- Produces:
  - `pub const METHODS: &[MethodSpec]` with `MethodSpec { name: &'static str, lifecycle: bool, params: fn(&mut SchemaGenerator) -> Schema, result: fn(&mut SchemaGenerator) -> Schema }`
    — one row per plan-1 method; plans 2 and 3 add rows. Lifecycle rows: `hello`, `rpc.discover`,
    `daemon.info`, `daemon.shutdown`.
  - `pub fn document() -> serde_json::Value`: OpenRPC 1.3 (`openrpc`, `info { title: "tdo", version: <crate version> }`,
    `x-protocol: 1`, `methods[]` with `params` as one by-name object param, `result`, `x-lifecycle`,
    `x-errors: [kinds]`), events under `x-events` (`event`, `missed`, `heartbeat`), shared definitions
    under `components.schemas`. Output sorted and pretty-printed with a trailing newline so diffs are stable.
  - `pub fn breaking_changes(released: &Value, current: &Value) -> Vec<String>` — one line per break:
    1. a released method or event is gone;
    2. a property of a released params/result/event object is gone;
    3. a params property is required now but was not (or is new and required);
    4. an enum (including `ErrorKind`) lost a value, or a params enum gained one is **not** a break
       but a result enum losing one is;
    5. a lifecycle method's params or result changed at all (frozen, spec §6.3).
    Checks follow `$ref`s into `components.schemas`, comparing by definition name.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn checked_in_schema_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/tdo.openrpc.json");
    let current = serde_json::to_string_pretty(&schema::document()).unwrap() + "\n";
    if std::env::var_os("TWYDO_BLESS").is_some() { std::fs::write(path, &current).unwrap(); }
    assert_eq!(std::fs::read_to_string(path).unwrap(), current,
        "schema/tdo.openrpc.json is stale: run TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema");
}

#[test]
fn no_breaking_change_against_the_released_schema() {
    let released = load("released.openrpc.json");
    assert_eq!(schema::breaking_changes(&released, &schema::document()), Vec::<String>::new());
}

#[test]
fn removing_a_result_field_is_a_break() { /* drop WriteResult.revision from a copy of document() → one line naming it */ }
#[test]
fn a_new_required_param_is_a_break() { /* add "x" to SubmitParams.required in a copy */ }
#[test]
fn a_new_optional_param_is_not_a_break() { /* add optional property → empty */ }
#[test]
fn a_lost_error_kind_is_a_break() { /* remove "stale" from ErrorKind enum */ }
#[test]
fn any_change_to_a_lifecycle_method_is_a_break() { /* add optional param to daemon.info */ }
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test schema` → FAIL.
- [ ] **Step 3: Implement** `schema.rs`; generate with `TWYDO_BLESS=1`, then copy
  `tdo.openrpc.json` to `released.openrpc.json` (nothing is released yet; the release pipeline step
  that refreshes it is plan 3's).
- [ ] **Step 4: Run** `cargo test -p twydo-daemon --test schema` → PASS.
- [ ] **Step 5: Commit** `feat(daemon): generate the OpenRPC schema and guard breaking changes`

---

### Task 4: Socket, single-instance lock, lifecycle methods and the client

**Files:**
- Create: `crates/twydo-daemon/src/paths.rs`, `src/server/mod.rs`, `src/server/conn.rs`, `src/client.rs`
- Modify: `crates/twydo-core/src/sync/mod.rs` (`pub mod lock;`, `LockFile` and `take` become `pub`)
- Test: `crates/twydo-daemon/tests/lifecycle.rs`, extend `tests/common/mod.rs`

**Interfaces:**
- Produces:
  - `paths::instance(vdir_flag: Option<&Path>) -> io::Result<Instance>` (reads no file),
    `paths::socket_path(flag: Option<&Path>) -> PathBuf` (reads `TWYDO_SOCKET` and the runtime
    variables; rules in "Decisions") and its pure form
    `paths::socket_path_from(flag: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>, uid: u32) -> PathBuf`; `paths::DaemonLock::take(state: &Path) -> io::Result<Option<DaemonLock>>`
    (`<state>/daemon.lock`, try-only).
  - `server::DaemonOptions { socket: PathBuf, matrix: Matrix, heartbeat: Duration /* 15 s */, event_buffer: usize /* 1024 */, session_ttl: Duration /* 600 s */, crash_after_commit: bool, before_recovery: Option<Box<dyn FnOnce() + Send>> }`
    with `DaemonOptions::new(socket)` filling the defaults.
  - `server::Daemon::start(cfg: Config, instance: Instance, opts: DaemonOptions) -> Result<DaemonHandle, StartError>`
    (the caller loaded `cfg` for `instance`: `tdo daemon` with today's `resolve_config`, tests directly);
    `StartError::AlreadyRunning` (lock held, or a live daemon answers on the socket), `StartError::Io`.
    Startup order (spec §7.1): take the lock → create the socket dir (`0700`) → remove a leftover
    socket only if connecting to it fails → bind, `chmod 0600` → start accepting (only lifecycle
    methods answer; others get `unavailable` "the daemon is starting") → `before_recovery` hook →
    `Store::open_with` (recovery) → load and publish the first snapshot → drain the queue (Task 8)
    → start the watcher (Task 9) → ready.
  - `DaemonHandle { epoch(), socket(), shutdown(&self), wait(self) }`; `shutdown` stops accepting,
    makes new requests `unavailable`, waits for a running write, closes connections, removes the
    socket, drops the lock. `Drop` shuts down.
  - `conn::peer_allowed(peer_uid: u32, own_uid: u32) -> bool` and `conn::peer_uid(&UnixStream) -> io::Result<u32>`
    (`SO_PEERCRED` on Linux, `getpeereid` on macOS, via `libc`).
  - Dispatch: requests before a successful `hello` may only call lifecycle methods (others:
    `invalid` "say hello first"). `hello` checks `protocol == PROTOCOL` (else `protocol` with
    `data.daemon` and `data.protocol`) and `params.instance == ` the daemon's instance (else
    `wrong_instance`), then creates or resumes the session (Task 7 fills sessions; here: a fresh random
    id `"s-" + 12 hex`).
  - `client::Client::connect(socket, HelloParams) -> Result<(Client, HelloResult), ClientError>`,
    `Client::connect_raw(socket) -> Result<Client, ClientError>`,
    `Client::call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R, ClientError>`
    (thread-safe; a reader thread routes responses by id and notifications to
    `Client::notifications(&self) -> &Receiver<Notification>`),
    `Client::next_request_id(&self) -> String` (`"<session>-<counter>"`),
    `ClientError::{Io, Rpc(RpcError), Closed, Decode(String)}`,
    `Notification::{Event(EventEnvelope), Missed(Missed), Heartbeat(Heartbeat)}`.
  - `tests/common`: `Fixture::instance()` = `Instance { config: <tempdir>/config.toml, vdir: Some(vdir) }`,
    `TestDaemon::start(fx: &Fixture) -> TestDaemon` (socket in the tempdir),
    `TestDaemon::start_with(fx, |opts| ...)`, `td.client() -> Client` (hello done, client `"test"`),
    `td.raw() -> Client`, `td.handle()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn hello_returns_epoch_session_and_vdir() { /* result.protocol == 1, session starts "s-", vdir == canonical */ }
#[test] fn protocol_mismatch_is_refused_with_both_versions() {
    let err = hello(&td.raw(), HelloParams { protocol: 2, ..ok() }).unwrap_err();
    assert_eq!(kind(&err), ErrorKind::Protocol);
    assert_eq!(err_data(&err).protocol, Some(1));
}
#[test] fn a_client_for_another_instance_gets_wrong_instance() { /* hello with instance.vdir = another tempdir → wrong_instance */ }
#[test] fn socket_flag_wins_over_env_which_wins_over_the_default() {
    // socket_path_from: flag → flag; no flag + TWYDO_SOCKET → env; neither → $XDG_RUNTIME_DIR/twydo/tdo.sock;
    // no XDG_RUNTIME_DIR (Linux) → /tmp/twydo-<uid>/tdo.sock
}
#[test] fn instance_reads_no_file() { /* paths::instance works with the config path pointing at a 0o000 file */ }
#[test] fn a_refused_instance_leaves_the_daemon_running() { /* wrong_instance hello, then daemon.info from a correct client still answers */ }
#[test] fn lifecycle_methods_answer_without_hello() {
    let c = td.raw();
    let info: DaemonInfo = c.call("daemon.info", json!({})).unwrap();
    assert_eq!(info.protocol, 1);
    let _: serde_json::Value = c.call("rpc.discover", json!({})).unwrap();
    assert_eq!(kind(&c.call::<_, Value>("snapshot.get", json!({})).unwrap_err()), ErrorKind::Invalid);
}
#[test] fn a_second_daemon_on_the_same_vdir_does_not_start() {
    assert!(matches!(Daemon::start(fx.config(), opts_other_socket()), Err(StartError::AlreadyRunning)));
}
#[test] fn a_stale_socket_file_is_replaced() { /* create a bound-then-dropped socket file at the path, start → ok */ }
#[test] fn requests_wait_for_recovery_as_unavailable() {
    // before_recovery blocks on a channel; snapshot.get → unavailable; release → snapshot.get ok
}
#[test] fn pipelined_requests_answer_by_id() { /* write 3 requests in one write_all on a raw UnixStream, read 3 responses, ids {1,2,3} */ }
#[test] fn malformed_input_does_not_hurt_other_clients() {
    // raw stream A sends "not json\n" → response with code -32700, connection stays open;
    // A sends a line over MAX_LINE → connection closed; client B's daemon.info still answers.
}
#[test] fn shutdown_removes_the_socket_and_releases_the_lock() { /* handle.shutdown(); path gone; Daemon::start again ok */ }
#[test] fn peer_check_refuses_other_users() { assert!(peer_allowed(1000, 1000)); assert!(!peer_allowed(0, 1000)); }
#[test] fn socket_dir_is_private() { /* mode of parent dir & 0o777 == 0o700, socket == 0o600 */ }
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test lifecycle` → FAIL.
- [ ] **Step 3: Implement** paths, server skeleton, connection threads (reader thread: `read_line`,
  parse, spawn one thread per request; writer thread: owns the write half, drains a
  `sync_channel(opts.event_buffer)` of lines; responses use blocking `send`), dispatch table, client.
- [ ] **Step 4: Run** `cargo test -p twydo-daemon --test lifecycle` → PASS.
- [ ] **Step 5: Commit** `feat(daemon): serve lifecycle methods on a private Unix socket`

---

### Task 5: Snapshot, status and events

**Files:**
- Create: `crates/twydo-daemon/src/server/publish.rs`, `src/server/bus.rs`
- Test: `crates/twydo-daemon/tests/read.rs`, `tests/events.rs`

**Interfaces:**
- Consumes: `to_wire` (Task 1), `EventEnvelope`/`Missed`/`Heartbeat` (Task 2).
- Produces:
  - `Publisher::reload(&self, cause: Cause, uids: Vec<String>) -> io::Result<u64>`: loads the vdir
    under `VdirLock::reader`, applies `store::queue::view`, and **publishes only if any task rev,
    collection or load error differs** from the current snapshot (then `revision += 1`, keep it in
    the ring of 32, emit `data.changed`). Returns the current revision either way.
  - `Publisher::current(&self) -> (u64, Arc<Snapshot>)`, `Publisher::at(&self, revision: u64) -> Option<Arc<Snapshot>>`.
  - `Publisher::refresh_status(&self) -> u64`: rebuilds `StatusView` from `sync::summary`,
    `Queue::unshown` (refusals with the item's `request`), the journal's pending count and the
    recovery report's stuck ids; bumps `status_rev` and emits `status.changed` only when the content
    (ignoring `status_rev`) changed. `sync.state` is always `idle` in this plan.
  - `EventBus::subscribe(&self, conn: ConnId, out: SyncSender<String>)`, `unsubscribe(conn)`,
    `emit(&self, Event)` (assigns `seq`, `try_send` to each; on `Full` marks the subscriber
    `behind`; the next emit to a `behind` subscriber tries to send `missed {seq, revision, status_rev}`
    instead of the event and clears the mark on success), `heartbeat thread` every
    `opts.heartbeat` sending `heartbeat {seq}` to each subscriber.
  - Methods: `events.subscribe {}` → `SubscribeResult`, `events.unsubscribe {}` → `{}`,
    `snapshot.get {}` → `WireSnapshot`, `status.get {}` → `StatusView`.

- [ ] **Step 1: Write the failing tests**

```rust
// read.rs
#[test] fn snapshot_has_tasks_collections_and_load_errors() { /* one bad .ics → errors[0].path == "inbox/bad.ics" */ }
#[test] fn snapshot_marks_queued_rows_pending() { /* push a queue item with Store under an external VdirLock before start → pending:true and overlaid summary */ }
#[test] fn status_reports_pending_edits_and_unacknowledged_refusals() { /* seed queue/refused/<id>.json with shown=false → refusals.len()==1, message == refusal_message */ }
#[test] fn status_has_no_sync_without_a_sync_table() { assert!(status.sync.is_none()); }

// events.rs
#[test] fn one_data_changed_per_published_revision() {
    // subscribe; Publisher reload twice with an outside change between → exactly one data.changed, revision+1
}
#[test] fn a_reload_without_changes_publishes_nothing() {}
#[test] fn stalled_subscriber_does_not_block_others() {
    // event_buffer = 4; subscriber A never reads its raw stream; trigger 20 outside changes;
    // subscriber B receives every data.changed; A eventually reads one "missed" whose seq >= the last emitted;
    // a write (Task 7 not yet) — here: daemon.info on A's connection still answers.
}
#[test] fn events_after_missed_continue_in_order() { /* after missed, the next events have seq > missed.seq, consecutive */ }
#[test] fn heartbeat_on_idle() { /* heartbeat = 50 ms → a heartbeat within 500 ms, seq == current seq */ }
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test read --test events` → FAIL.
- [ ] **Step 3: Implement.** `TestDaemon` gains `td.touch_outside(rel, text)` that writes and calls
  `handle.reload_for_test(Cause::Outside)` (a `#[doc(hidden)]` method; the watcher replaces it in Task 9).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(daemon): publish snapshots, status and change events`

---

### Task 6: Store hooks for a daemon: request records, queue-now, drained items

**Files:**
- Modify: `crates/twydo-core/src/store/journal.rs`, `store/queue.rs`, `store/mod.rs`
- Create: `crates/twydo-core/src/edit/describe.rs` (moved `op_title`, `success_message` from `tdo/src/write.rs`)
- Modify: `crates/tdo/src/write.rs` (call `describe`)
- Test: `crates/twydo-core/tests/store_requests.rs`

**Interfaces:**
- Produces:
  - `journal::RequestRef { session: String, request_id: String }` (serde, `Clone`, `Eq`);
    `Entry.request: Option<RequestRef>` and `Item.request: Option<RequestRef>`, both
    `#[serde(default, skip_serializing_if = "Option::is_none")]`.
  - `Store::set_request(&mut self, r: Option<RequestRef>)`: every entry or queue item the store
    creates until the next call carries it. A drained item's entry carries the item's `request`.
  - `Store::set_vdir_busy(&mut self, busy: bool)`: while true, `submit`, `undo`, `undo_last`,
    `redo_last` never try the vdir lock; they plan, gate and queue at once (and `settle_queued`
    skips its retry).
  - `Store::take_drained(&mut self) -> Vec<Drained>`: every item drained by this store's calls since
    the last take (today `submit` discards what its `drain_locked` returns).
  - `Store::without_recovery(cfg: &Config, matrix: Matrix) -> Result<Store, WriteError>` (today's
    `for_drain` with a matrix; `for_drain` calls it).
  - `Journal::by_request(&self, request_id: &str) -> io::Result<Option<Entry>>`;
    `Queue::by_request(&self, request_id: &str) -> io::Result<Option<Queued>>` with
    `Queued::{Pending(Item), Claimed(Item), Refused(Refusal)}`, and `Queue::state(&self, id: &str) -> io::Result<Option<Queued>>`
    for a queue id.
  - `edit::describe::op_title(snap: &Snapshot, op: &Op) -> String`,
    `edit::describe::success_message(snap: &Snapshot, op: &Op) -> String` (bodies moved unchanged).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn an_applied_entry_records_its_request() { /* set_request(Some(r)); submit → journal.get(id).request == Some(r); by_request finds it */ }
#[test] fn a_queued_item_and_its_drained_entry_record_the_request() { /* hold VdirLock::shared; submit → Queued(item) with request; drop lock; drain → entry.request == Some(r) */ }
#[test] fn a_refused_queued_item_is_found_by_request() { /* queue, then change the file outside, drain → Queued::Refused */ }
#[test] fn vdir_busy_queues_without_touching_the_lock() {
    // set_vdir_busy(true); submit with the lock FREE → Submitted::Queued, file unchanged, completes in < 100 ms
}
#[test] fn take_drained_returns_items_a_submit_drained() { /* queue one item, then a free-lock submit → take_drained().len()==1 */ }
#[test] fn entries_without_a_request_still_read() { /* a journal JSON without "request" deserializes; serialized entry without request has no "request" key */ }
```

- [ ] **Step 2: Run** `cargo test -p twydo-core --test store_requests` → FAIL.
- [ ] **Step 3: Implement.** Keep every existing signature; only add.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS (the TUI's write tests prove `describe` kept its wording).
- [ ] **Step 5: Commit** `feat(core): record requests on journal entries and queue items`

---

### Task 7: Sessions, the lane token and task writes with replay

**Files:**
- Create: `crates/twydo-daemon/src/server/lane.rs`, `sessions.rs`, `replay.rs`, `writes.rs`
- Test: `crates/twydo-daemon/tests/writes.rs`

**Interfaces:**
- Consumes: Task 5 `Publisher`, Task 6 store hooks and `describe`.
- Produces:
  - `Lane::write(&self) -> Result<WriteGuard, LongBusy>`: waits while another write holds the
    token, returns `LongBusy(kind)` at once while a long job holds it.
    `Lane::long(&self, kind: LongKind) -> LongGuard` waits for `Free`. `LongKind::{Sync, List, Test}`.
    Dropping a `LongGuard` runs the drain hook (Task 8) before freeing the token.
  - `Sessions::open(&self, resume: Option<&str>) -> String` (unknown or expired id → new session),
    `Sessions::get(&self, id) -> Option<Arc<Mutex<Session>>>`, `Sessions::attach/detach(id)` (count
    connections; a session expires `opts.session_ttl` after its last detach, checked lazily).
    `Session { store: Store /* without_recovery */, done: Vec<Titled>, undone: Vec<Titled> }`
    (`Titled` and the stack logic ported from `tdo/src/write.rs`).
  - `RequestLog::begin(&self, request_id) -> Begin` = `New(Ticket) | Done(Result<Value, RpcError>)`;
    a second `begin` of an in-flight id **waits** for the first to finish and returns its result.
    `Ticket::finish(self, result)`. On `New`, the caller first does the durable lookup
    (`Journal::by_request`, `Queue::by_request`) and, if found, finishes the ticket with the
    reconstructed result: journal entry → `applied` (`op_id` = entry id, current revision,
    `message` "already applied"); pending/claimed item → `queued` (`op_id` = item id); refusal →
    the refusal error (`stale` if `r.stale`, else `blocked`).
  - `op.submit`: replay → session (`not_found` "unknown session; reconnect") → epoch
    (`stale` "reload and try again" if not ours) → `Publisher::at(based_on.revision)` (missing →
    same `stale`) → lane: `WriteGuard` → `store.set_request`, `store.submit(view, op, label, "daemon", now)`,
    publish (`Cause::Op`, entry uids), refresh status; `LongBusy` → same with `set_vdir_busy(true)`
    → `queued`. Emit `op.finished` for everything `take_drained()` returns.
    `opts.crash_after_commit` → `std::process::abort()` after the commit, before replying.
  - `op.undo` / `op.redo`: replay → session → lane as above → `store.undo_last` / `redo_last`;
    `WriteResult.undid` names the title. `Submitted::Cancelled` (the undo removed a still-queued
    item) → outcome `applied` with `op_id` = that queue id. `NothingToUndo`/`NothingToRedo` →
    `invalid` with today's text.
  - `op.get`: by `request_id` (the `RequestLog`, then the durable lookup) or by `op_id`:
    `Journal::get` → `applied`; `Journal::by_queue_id` → `applied { op_id: entry id }`;
    `Queue::state` → `pending` / `refused`; a queue-format id with none of these → `cancelled`
    (only an undo removes an item without a record); else `not_found`.
  - `op.check`: gate verdict on `Publisher::at(based_on)` with the daemon's matrix and write mode,
    today's date in the snapshot's zone; never writes.
  - Error mapping `fn refusal(e: WriteError, view: &Snapshot, current: &Snapshot) -> RpcError`:
    `Blocked` → `blocked`; `Edit` → `invalid`; `Stale(path)` → `stale` with `uid` of the task at
    `path` in `view`, `current_rev` from `current` (None when deleted), message
    `"\"<title>\" changed on disk; reload and try again"` (today's TUI wording); `Stuck` → `stuck`;
    `Busy` → `unavailable`; `Io` → `internal`.

- [ ] **Step 1: Write the failing tests** (helpers in `tests/common`: `submit(&c, op) -> Result<WriteResult, ClientError>`
  using the current revision from `snapshot.get` and `c.next_request_id()`, `submit_at(&c, based_on, request_id, op)`,
  `undo(&c)`, `redo(&c)`, `kind(&err)`, `err_data(&err)`, `snapshot(&c) -> WireSnapshot`,
  `task(&snap, uid) -> &WireTask`, `create(title) -> Op`, `edit_title(uid, title) -> Op`,
  `count_titled(&td, title) -> usize` (files in the vdir whose SUMMARY is `title`))

```rust
#[test] fn a_create_is_applied_and_published() {
    let r = submit(&c, Op::Create(NewTask::plain("inbox", "Synthetic new"))).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    assert!(r.created.is_some());
    let snap: WireSnapshot = c.call("snapshot.get", json!({})).unwrap();
    assert_eq!(snap.revision, r.revision);
    assert!(snap.tasks.iter().any(|t| t.task.summary == "Synthetic new"));
}
#[test] fn the_answer_carries_todays_message() { /* complete a task → message == "done" */ }
#[test] fn a_resent_request_returns_the_first_result_and_applies_once() {
    let id = c.next_request_id();
    let a = submit_at(&c, based, &id, create("X")).unwrap();
    let b = submit_at(&c, based, &id, create("X")).unwrap();
    assert_eq!(a.op_id, b.op_id);
    assert_eq!(count_titled(&td, "X"), 1);
}
#[test] fn replay_comes_before_the_session_check() {
    // apply with session S; resend same request_id with session "s-unknown" → same op_id, no error
}
#[test] fn concurrent_resend_applies_once() {
    // two clients, same request_id, two threads released by a Barrier → identical op_id, one task created
}
#[test] fn a_write_from_another_epoch_is_stale() { /* based_on.epoch = "d-old" → stale, message contains "reload and try again" */ }
#[test] fn a_revision_no_longer_kept_is_stale() { /* 33 outside changes, then based_on = first revision → stale */ }
#[test] fn an_edit_of_a_task_changed_since_based_on_is_stale_with_its_current_rev() {
    // based_on = r; outside edit of u1 (published r+1); edit u1 based on r → stale, data.uid=="u1", current_rev == rev in r+1
}
#[test] fn a_family_operation_is_refused_when_a_member_was_deleted_since_based_on() {
    // project p1 with c1,c2 at revision r; remove c2 outside (r+1); Complete{p1} based_on r → stale.
    // Planning on r+1 would have succeeded: assert Complete{p1} based_on r+1 is Applied afterwards.
}
#[test] fn a_family_write_changes_rows_the_client_did_not_name() {
    // Complete{p1} (test mode) → re-fetched snapshot shows c1 and c2 completed; data.changed.uids include them
}
#[test] fn undo_is_per_session() {
    // client A edits u1, client B edits u2; A undo → u1 restored, u2 still edited; B undo → u2 restored
}
#[test] fn a_resent_undo_does_not_undo_a_second_edit() {
    // A edits u1 then u2; undo with request_id X → u2 restored; resend X → same op_id, u1 still edited
}
#[test] fn a_session_survives_a_reconnect() { /* A edits; drop A; reconnect with hello.session = old id; undo works */ }
#[test] fn a_session_expires_after_its_ttl() { /* session_ttl 100 ms; reconnect after 300 ms → new session id; undo → invalid "nothing to undo" */ }
#[test] fn the_gate_refusal_is_blocked_with_the_gates_reason() { /* writes.mode verified + an unverified op → blocked */ }
#[test] fn op_get_reports_an_applied_request() { /* op.get {request_id} → applied {op_id} */ }
#[test] fn op_check_never_writes() { /* op.check on a delete → allowed, file still there */ }
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test writes` → FAIL.
- [ ] **Step 3: Implement** lane, sessions, replay, writes; `hello` now opens or resumes sessions.
- [ ] **Step 4: Run** `cargo test -p twydo-daemon --test writes` → PASS.
- [ ] **Step 5: Commit** `feat(daemon): apply task writes with sessions, replay and based_on`

---

### Task 8: The queue inside the daemon, drains, refusals and acknowledgement

**Files:**
- Modify: `crates/twydo-daemon/src/server/lane.rs`, `writes.rs`, `server/mod.rs`
- Test: `crates/twydo-daemon/tests/queue.rs`

**Interfaces:**
- Consumes: Task 7 `Lane`, `LongGuard`, `Sessions`; Task 6 `take_drained`.
- Produces:
  - `DaemonHandle::hold_long(&self, kind: LongKind) -> LongGuard` (the real entry point plan 2's
    sync and list jobs use; tests use `LongKind::Test`).
  - Drain hook on `LongGuard` drop, at startup (Task 4 order) and nowhere else: a
    `Store::without_recovery` drains; for each `Drained`, emit
    `op.finished { op_id: item.id, request_id, state }` (`Applied`/`AlreadyApplied` →
    `applied { op_id: entry id }`, `Refused(reason)` → `refused { kind: stale | blocked, message: refusal_message }`);
    for a refusal, drop the matching `Titled` from the owning session's stacks (by queue id); then
    publish (`Cause::Op`) and refresh status. A drain error that leaves items (stuck journal, I/O)
    is reported in status and retried at the next release.
  - `refusals.ack { request_id, ids }`: replay (in-memory only: the record is the refusal itself),
    `Queue::mark_shown` for each id, refresh status → `{ acknowledged }`. Unknown ids are ignored.
  - `op.get` for a queued `op_id` returns `pending` until drained, then the drained state, also after
    a daemon restart (the record is on disk).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn an_edit_during_a_long_job_is_queued_at_once_and_shown_pending() {
    let job = td.handle().hold_long(LongKind::Test);
    let t = Instant::now();
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert!(t.elapsed() < Duration::from_millis(200), "never waits for the lane");
    let snap = snapshot(&c);
    let u1 = task(&snap, "u1");
    assert!(u1.pending); assert_eq!(u1.task.summary, "Renamed");
    assert_eq!(fx.read("inbox/u1.ics").contains("Renamed"), false);
    drop(job);
}
#[test] fn the_drain_after_the_job_applies_and_reports_op_finished() {
    // subscribe; hold; submit → queued(op_id Q); drop guard → op.finished {op_id: Q, request_id, state: applied}; file renamed; pending false
}
#[test] fn a_queued_edit_refused_at_drain_stays_in_status_until_acknowledged() {
    // hold; submit edit u1; outside change of u1; drop → op.finished refused(stale);
    // status.refusals has it (session still alive); refusals.ack → status.refusals empty
}
#[test] fn undo_of_a_still_queued_edit_removes_it() { /* hold; submit (queued Q); undo → outcome applied, op_id == Q, message "undone: <title>", queue empty; drop → file unchanged; op.get {op_id: Q} → cancelled */ }
#[test] fn undo_skips_a_refused_queued_edit() {
    // edit u2 applied ("U2"); hold; edit u1 queued; outside change u1; drop (refused);
    // undo → undid.title == "U2"'s pre-edit title and u2 restored
}
#[test] fn op_get_follows_a_queued_op_to_its_outcome() { /* pending while held, applied after drop */ }
#[test] fn queued_items_left_by_an_earlier_daemon_drain_at_startup() {
    // push an item with Store under an external VdirLock::shared; start daemon after dropping the lock
    // → file changed, op.get {request_id} applied
}
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test queue` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p twydo-daemon` → PASS.
- [ ] **Step 5: Commit** `feat(daemon): queue writes during long jobs and drain them after`

---

### Task 9: Watcher, the hidden `tdo daemon` command and crash tests

**Files:**
- Create: `crates/twydo-daemon/src/server/watch.rs`, `crates/tdo/src/cli/daemon.rs`, `crates/tdo/tests/daemon_process.rs`
- Modify: `crates/tdo/Cargo.toml` (`twydo-daemon`, `signal-hook = "0.3"`), `crates/tdo/src/main.rs`,
  `crates/tdo/src/cli/mod.rs`, `crates/twydo-daemon/Cargo.toml` (`notify = "8.2.1"`)
- Test: `crates/twydo-daemon/tests/events.rs` (watcher cases), `crates/tdo/tests/daemon_process.rs`

**Interfaces:**
- Consumes: `Publisher::reload` (Task 5), `DaemonOptions` (Task 4).
- Produces:
  - `watch::start(vdir: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher>`:
    the `is_change` filter and 300 ms quiet debounce moved from `tdo/src/term.rs` (the TUI keeps its
    own copy until plan 3 removes it). The callback calls `Publisher::reload(cause, vec![])` where
    `cause` is `Sync`/`List` while a matching `LongGuard` is held, else `Outside`. Uids are the
    tasks whose rev changed.
  - Global flag `--socket <PATH>` on `tdo` (hidden for now), resolved with `paths::socket_path`. `tdo daemon` builds its instance with
    `paths::instance(cli.vdir)` and its config with today's `resolve_config(cli.vdir)`; a missing or
    invalid config exits with `EXIT_ERROR` and today's message.
  - `tdo daemon` (hidden subcommand): `--matrix <PATH>` (hidden, as `tdo op`),
    `--test-crash-after-commit` (hidden). Runs in the foreground, logs one line per start, stop and
    error to stderr, exits `EXIT_ALREADY_RUNNING` (10) with "another daemon serves <vdir>" when
    `StartError::AlreadyRunning`, and shuts down cleanly on `SIGTERM`/`SIGINT` (`signal-hook`
    flag polled by the main thread, then `handle.shutdown()`).

- [ ] **Step 1: Write the failing tests**

```rust
// twydo-daemon/tests/events.rs
#[test] fn a_hand_edit_is_published_with_cause_outside() {
    // subscribe; fx.write("inbox/u1.ics", changed) → within 2 s data.changed {cause: outside, uids: ["u1"]}
}
#[test] fn own_write_publishes_one_revision() {
    // subscribe; submit edit → exactly one data.changed (cause op) within 1 s after the reply; revision == result.revision
}

// tdo/tests/daemon_process.rs (spawns env!("CARGO_BIN_EXE_tdo") --vdir <fx> daemon with TWYDO_SOCKET=<tmp>/tdo.sock)
#[test] fn the_daemon_listens_on_twydo_socket() { /* socket appears at TWYDO_SOCKET; --socket <other> wins when both given */ }
#[test] fn a_second_daemon_exits_with_the_already_running_code() { /* status.code() == Some(10) */ }
#[test] fn sigterm_shuts_down_and_removes_the_socket() {}
#[test] fn a_resend_after_a_crash_between_commit_and_reply_returns_the_first_outcome() {
    // daemon with --test-crash-after-commit; submit create request R → connection closes, process exits (abort);
    // restart without the flag; resend R (same session id is unknown now) → applied, same journal id; exactly one task "Synthetic crash"
}
#[test] fn a_queued_item_survives_kill_and_is_applied_once_after_restart() {
    // external VdirLock::shared held by the test (as a running sync would) → submit is queued (250 ms try);
    // kill -9 the daemon; drop the external lock; restart → op.get {request_id} applied; one change, once
}
```

- [ ] **Step 2: Run** `cargo test -p twydo-daemon --test events && cargo test -p tdo --test daemon_process` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` → PASS; `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- [ ] **Step 5: Commit** `feat(daemon): watch the vdir and add the hidden tdo daemon command`

---

### Task 10: Record progress

**Files:**
- Modify: `AGENTS.md` (progress note: daemon plan 1 done, plans 2 and 3 next, clients not switched),
  `docs/architecture.md` (one short "In progress: the daemon" paragraph pointing at the spec; full docs come with plan 3)
- Create: `docs/superpowers/plans/2026-11-09-daemon-2-sync-and-jobs.md` is **not** written here; it is the next plan.

- [ ] **Step 1:** Update the two files; `python3 tools/check_links.py` → passes.
- [ ] **Step 2:** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` → all pass.
- [ ] **Step 3: Commit** `docs: record daemon plan 1 progress`
