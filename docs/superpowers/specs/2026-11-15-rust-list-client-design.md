# Rust list client: remove the Python wrapper around vdirsyncer

Issue: #86 (phase 1 only). Status: awaiting approval.

## Intent

Twydo manages lists (create, rename, recolour, delete, discover) through
`crates/twydo-core/src/lists/helper.py`, a Python program bundled in the binary. It imports
vdirsyncer's internal modules, pinned to `0.21.x`, and patches one of its functions at runtime.
That API is not stable, and it forces a Python interpreter setting (`[sync].list_python`) on
the user.

This change deletes the wrapper. Twydo talks to the CalDAV server for list operations itself,
in Rust. vdirsyncer stays a runtime requirement for item sync, used only as a command line
tool.

## Goals

- `helper.py` is gone, along with every piece of code, configuration, documentation and test
  that exists only because of it.
- No Twydo code imports, embeds or runs Python at runtime.
- The only dependency on vdirsyncer is its command line: `sync`, `metasync` and `discover`.
- List behaviour seen by the user and by the server is unchanged: the same operations, the
  same guards, the same refusals, the same recovery.

## Non-goals

- Replacing vdirsyncer for item sync (#86 phases 2 and 3), or adopting pimsync, `libdav` or
  `vstorage`. That decision is deferred until item sync is designed.
- Changing the conflict hook, `sync/output.rs`, `sync/reconcile.rs` or the write queue.
- Porting the development scripts in `tools/` away from Python.
- New list features.

## Anti-goals

- A Rust port that keeps a subprocess, a JSON request/reply protocol, or an interpreter
  setting. The helper process and its protocol go; they are not re-created in Rust.
- Tests that keep a Python fake of vdirsyncer alive.
- Any path that can create a list on the server during discovery.
- Weaker guards than today. Where this document does not mention a check the helper makes,
  the check stays.

## Constraints

- Spec §11 preservation, conflict and recovery contracts apply unchanged. List writes stay
  best effort under the accepted risk recorded in the
  [best-effort list writes design](2026-11-12-best-effort-list-writes-design.md).
- An operation record written before this change (`lists/operations/<id>.json`, format 1,
  which embeds the helper request) stays readable, but reconciling it is refused: the record
  carries no fingerprint of the account the operation started on that the new client can
  compare (owner's decision of 2026-11-16).
- No device access is authorised by this document. See [Server check](#server-check).
- Twydo is MIT. New dependencies must be permissive.

## Approaches

Considered for the HTTP and CalDAV part:

1. **`ureq` and `roxmltree`, hand-written requests (chosen by the user).** A blocking HTTP
   client and an XML parser. The helper already builds its own XML for all server calls, so
   this is close to a line-for-line port and the requests on the wire stay nearly the same.
   Calendar-home discovery is new code.
2. **`libdav`.** Ready-made discovery and property calls, but it needs tokio and hyper, the
   TLS client has to be wired by hand, and its requests differ from the ones the evidence was
   recorded with. Too much machinery for four request types; revisit with item sync.
3. **`reqwest` (blocking), `curl` as a command, or a small CalDAV crate.** `reqwest` pulls in
   tokio for no gain; `curl` swaps one external requirement for another; small CalDAV crates
   were not checked and would give less control over the guards.

Also ruled out: swapping the vdirsyncer binary for pimsync. It would not remove the wrapper
problem and brings its own migration.

## Design

### Shape

The helper is a separate process because it had to be Python. In Rust it becomes ordinary
function calls inside the process that runs list operations today.

- **`lists/dav.rs` (new): the server calls.** One function per call the helper makes, with the
  same request bodies, headers and response checks. It knows nothing about operation records
  or locks.
- **`lists/remote.rs`, `lists/backup.rs`, `lists/operation.rs`: the guards and phases.** The
  code that today builds a `HelperRequest` and calls `helper::invoke` instead takes the locks,
  checks the operation record, writes the phase, calls `dav`, and writes the outcome.
- **`sync/discover.rs`: discovery,** rewritten as described under [Discovery](#discovery).
- **`sync/vdirsyncer.rs`: the command line,** see
  [The vdirsyncer boundary](#the-vdirsyncer-boundary).

`lists/helper.rs`, `lists/helper.py` and `sync/list_python.rs` are deleted.

### Server calls

Ported from the helper with unchanged semantics:

| Call | Used for |
|---|---|
| `PROPFIND` depth 0, named properties | Read name, colour, resource type and component set; require a calendar that supports `VTODO` |
| `PROPFIND` depth 0, `allprop` | Raw properties for a delete backup |
| `PROPFIND` depth 1, etag and resource type | Inventory for a delete backup |
| `GET` with `If-Match` | Each resource for a delete backup |
| `PROPPATCH` | Rename and recolour, with the before and after reads |
| `MKCALENDAR` with `If-None-Match: *` | Create, after checking the collection is absent |
| `DELETE` | Delete, after revalidating against the backup |

New, replacing what vdirsyncer's classes did:

- **Calendar home.** From `[sync].url`: current user principal, then calendar home set. A
  created list's address is the home plus the collection id.
- **Listing the home** (`PROPFIND` depth 1) to find an existing collection by id, and for
  discovery. A collection's id is the last path segment of its address, decoded, as
  vdirsyncer derives it.

Rules that apply to every request, all carried over from the helper:

- Stay on the configured origin; never follow a redirect; a 3xx is a failure.
- Validate every target address as the helper's `validate_target` does.
- Cap a response body at 4 MiB and a backup at 32 MiB in total.
- Never put server output, addresses or credentials in an error message. Errors keep the
  helper's fixed set of codes and wordings, minus the ones that only described the helper
  itself (`dependency`, `protocol`).

Connection, from `[sync]`: basic authentication with `username` and the output of
`password_command` (trailing newlines removed, its stderr discarded); TLS trusting `ca_file`
when set and the system's roots otherwise; the user agent `tdo/<version>`. The password
command runs once per operation and only after the operation's local checks pass, as today.

### Guards, phases and locks

Everything the helper's `operation_guard`, `discover_guard` and `main` do moves into Rust with
the same order and meaning:

- Take the exclusive sync run lock and the shared vdir lock for the whole operation.
- Check the operation record against the request, the active id, the bound vdir and the local
  snapshot, with the same per-mode rules.
- Write `remote-started` before the first server call, then `remote-confirmed` with the
  receipt, `local-confirmed` after a refresh, or `uncertain` on failure with the same
  `no_remote_write`, `present` and `reason` receipts.

Three things differ because there is no child process:

- **Locks are held by the process itself.** They are released if it is killed. The record then
  still says `remote-started`, which is already treated as an uncertain outcome to reconcile.
- **Deadline.** Each request has a timeout, and the operation checks an overall deadline of
  `[sync].timeout_secs` between requests. On expiry it records `uncertain` and fails with the
  current timeout wording.
- **Stopping.** A list operation is not interrupted by a stop request, as before this change
  (the helper ran unwatched): it ends at its deadline at the latest. A stop request applies to
  discovery, which checks it before listing the server, again before running
  `vdirsyncer discover`, and while that command runs.

The operation record keeps format 1 and keeps reading the fields it has today. Fields that
only addressed the helper (`pair`, `local_storage`, `config`, `version`) are accepted and
ignored when read. A record written before this change has no `connection` fingerprint, so a
reconcile of it is refused with the code `upgrade` (before any server call or password
command, nothing sent), by the owner's decision of 2026-11-16: the old record carries no
fingerprint of the account that the new client can compare. A record with no remote target
is still cleared as never sent. No command resumes or abandons the others.

### Discovery

Today the helper calls vdirsyncer's internal discovery and replaces its "collection not
found" handler, because the stock handler either asks on the terminal or creates the missing
side. The replacement keeps that handler from being reached:

1. Under the locks the sync run already holds, list the calendar home.
2. A local folder with no server collection: stop with `DiscoverError::Missing` for that id.
   `vdirsyncer discover` is not run. One list is reported per run, as today.
3. A server collection with no local folder: create the empty folder and record it as adopted.
4. Run `vdirsyncer discover` through the vdirsyncer boundary, with no terminal input.
5. If step 4 fails, remove the folders step 3 created, only while empty, and report the
   failure. The previous cache stays.

The generated vdirsyncer config must never contain `implicit = "create"`. With no terminal
input, a "collection not found" that is reached anyway (a list deleted between steps 1 and 4,
or a difference between Twydo's listing and vdirsyncer's) fails the prompt and aborts
discovery without creating anything. This backstop is a contract and needs a test that runs
the real `vdirsyncer` command.

The existing fallback that runs plain `vdirsyncer discover` when the `sync/rediscover` marker
is set stays as it is.

### The vdirsyncer boundary

Nice to have, and the last step so it can be dropped without affecting the rest.

`sync/vdirsyncer.rs` becomes the only place that knows vdirsyncer's command line: the program
name, the subcommands, the pair name, `VDIRSYNCER_CONFIG` and the generated config. It offers
three operations to the rest of the code: discover, sync (optionally forcing named lists) and
metasync. `sync/run.rs` and `sync/discover.rs` call those instead of building argument lists.

The process handling in `run_command` (locks on stdin, process group, timeout, stop) and the
parsing in `sync/output.rs` stay where they are. Reading vdirsyncer's output is still
vdirsyncer-specific; hiding that too belongs to the item-sync work.

### Configuration and checks

- `[sync].list_python` is removed, following how #88 handled its removed keys: it joins
  `REMOVED_SYNC_KEYS` in `config.rs`, so a config that still has the line fails to load with
  the `run tdo config init` hint, and `tdo config init` deletes the line. This makes the
  release breaking (`0.5.0`), as #88 was.
- `tdo config check` loses the `list helper` step.
- The protocol schema is regenerated.
- This closes #95 (showing the interpreter the daemon resolved): there is no interpreter left
  to resolve. The PATH difference between `tdo config check` and the daemon still applies to
  `[sync].command`; the note in `docs/config.md` stays for that key only.

### Tests

Removed, because they test the wrapper or exist to fake it:

- `crates/twydo-core/tests/lists_helper.rs`
- `tools/tests/test_list_helper.py`
- `crates/twydo-core/tests/common/list_backend.py`
- the interpreter-resolution tests in `sync/list_python.rs` and the `list helper` cases in
  `crates/tdo/tests/config_check.rs`


`mise run test` keeps its Python step for the other development-script tests in
`tools/tests`; only the helper's test file goes.

Kept, and pointed at a new fake: `lists_remote.rs`, `lists_operations.rs`,
`lists_recovery.rs`, `sync_discover.rs`, and the list tests in `twydo-daemon` and `tdo`. They
test Twydo's own behaviour and must keep passing with their assertions intact, apart from
wording that named the helper.

The fake is a small CalDAV server written in Rust for tests only, listening on localhost over
plain HTTP, with the fault switches the Python fake has today (`fail_status`, `ignore_write`,
`fail_delete_once`, `mutate_before_delete`, the inventory faults, the discovery scripts). It
runs out of process reach, so the tests that start a real `tdo` and daemon can use it too.

The checks the deleted Python tests covered (strict multistatus parsing, inventory
validation, colour comparison, target validation, request limits) remain contracts. They get
Rust tests against `lists/dav.rs`; they are rewritten, not ported one to one.

Fixture `fixtures/list-create/phone-confirmation/case.toml` names a test in
`lists_remote.rs`; keep that name valid or update the reference.

### Documentation

Update `docs/sync.md`, `docs/config.md`, `docs/install.md`, `docs/architecture.md`,
`docs/testing.md` and `AGENTS.md` so none describes a helper, an interpreter or a vdirsyncer
version pin. `python3 tools/check_links.py` must pass.

## Server check

The Rust client's requests will differ slightly from the helper's (header order, XML
whitespace, user agent, and the new calendar-home requests). No existing capture shows the
Synology server accepting them.

Before release, one bounded round on the isolated test list is proposed: create a list,
rename and recolour it, preview its deletion, delete it, and run one discovery. It needs the
user's explicit approval at that point; this document does not grant it. Until it passes, the
change is not released.

## Left to the builder

- Module boundaries inside `lists/`, and whether the fake server lives in a test-support
  crate or a shared test module.
- How the test seam `TWYDO_TEST_REMOTE_DIR` (a filesystem remote for sync tests) answers the
  home listing in discovery.
- The `ureq` and TLS feature set, as long as system roots and a custom CA file both work.
- The exact form of the vdirsyncer boundary (a struct, or a trait if tests benefit).

## Decisions made by the author, for the user to check

- In-process calls replace the helper process; locks are held by the daemon process itself.
- A list operation is not stoppable and ends at its deadline; a stop request applies to
  discovery only.
- Old operation records stay readable (format 1), but an operation started before this change
  is refused on reconcile, by the owner's decision of 2026-11-16, because the old record
  carries no fingerprint of the account the new client can compare.
- A leftover `list_python` line is an error fixed by `tdo config init`, so the release is
  breaking. The alternative is to accept and ignore the line, which no other key does.
- Discovery fails safely through the no-terminal-input backstop rather than by any new guard
  inside vdirsyncer.
- The fake server is a real localhost HTTP server rather than an in-memory substitute.
- The Server check gate.
