# Discovering lists created on the phone (issue #76)

## Why

Verified mode refuses list creation because the `list-create.collection` contract has
`iphone_to_twydo = "unverified"`. Issue #76 asks for a device round that creates a list on
the iPhone and confirms that Twydo picks it up with the right identity, name, color and tasks.

That round would fail today, for a product reason rather than a 2Do one: Twydo has no path
that picks up a list created elsewhere.

- `tdo sync` runs `vdirsyncer sync` and `metasync`, which only use the collection set cached
  by the last `vdirsyncer discover`. That set was created by hand at setup
  (`yes | vdirsyncer discover <pair>`). A new server collection is silently ignored.
- The bundled list helper (`crates/twydo-core/src/lists/helper.py`, `refresh_discovery`)
  refreshes the cache only for Twydo's own list creation, and refuses on purpose when any
  collection is missing on either side.
- The `sync/rediscover` step after `:forget-list` runs plain `vdirsyncer discover` and never
  answers its "create it?" prompt, so a new phone list makes that step fail and block sync.

## What the user wants

Create a list on the iPhone, and it appears in Twydo. The user chose (2026-11-11):

- **Discovery at daemon start.** The startup sync refreshes the collection set first.
- **A manual command** that forces discovery on demand, next to sync.
- **Ordinary syncs stay as they are**: periodic, batched and plain manual syncs do not
  discover and make no extra server request. A phone list therefore appears after the next
  daemon start or manual discovery, not after the next ordinary sync. This trade-off was
  accepted.

## Goals

1. A collection that exists on the server and not locally is adopted: a local folder is
   created, and the same run then syncs and metasyncs it, so it shows up with its tasks and
   with its name and color.
2. Discovery runs at the daemon's startup sync and on the manual command, nothing else.
3. The manual command discovers and then syncs in one step, reporting the result like a sync.
4. Device round, after the product change is merged-ready: a bounded, approved synthetic
   phone-created list is discovered by Twydo. The result is recorded as a fixture case, and
   `list-create.collection` gets `iphone_to_twydo = "pass"` only if the round passes.

## Non-goals

- Name/color editing and list deletion. Their `atomicity` failures are unchanged; nothing
  from this round promotes them.
- Archive state. Archive remains local.
- Discovery on every sync, or on TUI/CLI start.
- Changing accounts, global write mode, or other data.

## Anti-goals (failure even if it "works")

- **Creating a collection on the server** because a local folder has no server counterpart.
  That folder usually means the list was deleted on the phone; recreating it would undo the
  deletion. Only the local side may be created.
- Answering vdirsyncer's prompts wholesale (for example piping `yes`), which would approve
  both directions.
- A failed discovery blocking ordinary sync. Today's `sync/rediscover` gate after
  `:forget-list` stays as it is; startup and manual discovery must not add a new blocker.
- Touching the user's private lists, backups or session state during tests.

## Behavior

**Policy.** During discovery, a collection missing on the **local** (filesystem) side of the
configured pair is created there. A collection missing on the **remote** (CalDAV) side is
never created. It is reported and goes through the existing Not Found → `:forget-list` flow.

**Failure.** If discovery fails (server unreachable, helper unavailable, a remote-side missing
collection, timeout), the run reports it in `:errors` and the CLI report, keeps the previous
cached collection set, and continues with the ordinary sync.

**Stale local list (user's decision).** A local folder whose server collection is gone (a list
deleted on the phone, not yet `:forget-list`ed) makes the whole discovery stop, as vdirsyncer
does; no new list is adopted in that run. The report names the stale list and its collection
id and gives the `:forget-list` command, as Not Found reports do today. Working around
vdirsyncer to adopt the other lists anyway is out of scope.

**Exit code and status** *(my call)*. A discovery failure counts as a problem in the run:
`tdo sync --discover` exits 1 even if the ordinary sync that followed succeeded, and the
failure appears as an error in `:errors` and `tdo status`. A failed startup discovery is a
warning on the startup sync, not a failure, and does not stop the daemon: a pair layout the
helper refuses must not turn every daemon start into a failed sync.

**Forgotten lists.** A list removed locally with `:forget-list` that still exists on the
server is adopted again by the next discovery. `docs/sync.md` already says so.

**Rediscover after `:forget-list`** *(my call; revised in review)*. When the `sync/rediscover`
gate is pending, the local-only discovery runs first. If it succeeds it has refreshed the same
cache, so the gate is satisfied: the marker is removed and the plain `vdirsyncer discover` is
skipped. If the helper fails or cannot run (a warning), the gate's plain discovery runs exactly
as today, with its documented semantics: backup and request kept, ordinary sync blocked until
discovery succeeds. The gate and its tests keep working with non-CalDAV pairs the helper
refuses.

**Review rulings.** An unavailable helper (interpreter does not start, vdirsyncer not
importable) is only a warning, and is not stored in `status.discovery_error`, unless the
discovery was explicitly requested. A failed discovery rolls back the empty folders it created.
The helper takes the shared vdir lock itself and is recorded in `running.json`, so it can be
stopped and cannot outlive a killed daemon holding no lock. Only the first missing list is
reported per run (each deletion is confirmed separately). A refusal on a local storage that is
not a plain filesystem storage is a `mapping` problem, not a missing list. The startup discovery
is a durable `sync/discover` marker written when the daemon starts; the first run that gets past
the skip checks discovers and removes it, success or failure. The report and the stored status
use one message, built once with the list's display name.

## Commands (user's decision)

- CLI: `tdo sync --discover`. Output as `tdo sync`; exit codes as above.
- TUI: `:discover`. Behaves like `:sync`, plus discovery, and reloads the lists. Bare
  `:sync!` keeps its usage error.
- Daemon RPC: a sync request with a discovery flag. At startup the daemon leaves a
  `sync/discover` marker instead of flagging the startup sync (see Review rulings).

## Helper shape

Discovery runs in the bundled helper, not as the `vdirsyncer discover` command (user's
decision). The helper calls the same library function the command uses,
`collections_for_pair(..., from_cache=False)`, with `handle_collection_not_found` replaced:
missing locally → vdirsyncer's own `create_collection` on the local filesystem storage;
missing on the server → refuse. vdirsyncer then writes its usual collection cache. The plain
command was rejected because its single "create it?" prompt cannot tell the sides apart, and
piping `yes` would recreate lists deleted on the phone. Pre-creating local folders and then
running the plain command was rejected because it still needs the helper's server access to
list collections, plus a second discovery pass.

*(My call:)* Discovery is a new helper action (for example `discover`) that needs no list-operation
record, collection id or `state/lists/active` phase, unlike `create`/`edit`. It keeps the
helper's connection checks (same-origin, no redirects, status path outside the vdir, no
`url.fetch`/`path.fetch`, filesystem/CalDAV pair) and runs inside the sync job, holding the
vdir lane and sync run lock like the rest of the run. A pair layout the helper refuses is
reported as a discovery failure. Background syncs skip while automatic sync is off or
backing off; the startup discovery then waits in its marker for the next run that is not skipped,
and the manual command remains.
Both the test and production pairs use
`collections = ["from a", "from b"]`; an explicit collection list simply adopts nothing new.

## Constraints

- Existing locking: discovery writes vdirsyncer's cache under the vdir lane and must not race
  a running sync or list operation.
- The helper's existing safety checks stay: same-origin requests only, no redirects, status
  path outside the vdir, no `url.fetch`/`path.fetch`, filesystem/CalDAV pair only.
- Configured timeout and backoff apply to the discovery step as to sync.
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; no cargo target in `/tmp`.
- Docs to update: `docs/sync.md` (when lists are discovered, the command, the policy),
  `docs/tui.md`, `docs/compatibility.md` (collection management), `docs/reference/evidence.md`
  after the round. `python3 tools/check_links.py` must pass.

## Device round (phase 2, needs fresh approval)

Only after the product change is complete and tested locally. Before any phone/NAS action,
present an exact synthetic checklist and get approval. Expected shape:

1. On the iPhone, test account: create one new list with a synthetic name and a non-default
   color, plus one open and one completed synthetic task.
2. Sync the phone. Restart the test daemon or run the manual discovery command.
3. Record what Twydo shows: list present, name, color, both tasks, local folder identity
   matching the server collection. Record what the phone shows.
4. Capture the result with `tools/capture_fixture.py` as `fixtures/list-create/<case>/`,
   update `compat/matrix.toml` and the evidence/compatibility docs only for what was
   established.

No cleanup of the test list without separately agreed scope. Do not reuse the deleted
lifecycle collection or touch the archive test list.

## Left to the builder

- How the local-only policy is implemented inside the new helper action: most likely a
  replacement for `handle_collection_not_found` that creates only on the configured local
  filesystem storage and refuses otherwise.
- RPC/protocol shape for the discovery flag, schema updates, and status/report wording.
- Test design. At minimum: a remote-only collection is adopted and synced; a local-only folder
  is not created remotely and is reported; discovery failure leaves the ordinary sync running;
  ordinary syncs do not discover.
