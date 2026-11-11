# Issue 69: list management

Date: 2026-11-08 · Status: approved by the user; bounded archive investigation complete.

## Intent and decisions

Manage lists inside Twydo: create with name/color, edit name/color, archive/unarchive,
and delete. [Issue 69](https://github.com/bwilczynski/twydo/issues/69) supplies the scope.
The user explicitly chose deletion from CalDAV and all devices, with confirmation and
recovery, and a vdirsyncer helper rather than native Rust CalDAV. Work on branch
`feat/69-list-management` in the existing checkout; no worktree.

This supersedes the main spec's exclusion of collection deletion. The
[add-and-reshape design](2026-11-03-tui-add-and-structure-design.md) §5 already anticipates
sidebar list management and identifies its unverified collection semantics.

The requested [device archive check](../../reference/evidence.md)
observed no changes to either task, resource ETags or any of the 14 returned collection
properties through archive/unarchive and sync. Use Twydo's existing local `archived`
preference and label archiving as applying to this desktop only. Do not claim shared
archive state or a helper-writable archive property. The phone's observed unarchive path
is Settings → Privacy → Archived Lists.

## Approaches

1. **Chosen: narrow helper using the configured vdirsyncer installation.** Reuse its
   connection configuration, credential fetching, authentication and TLS handling for
   collection discovery, creation, metadata and deletion. Rust owns validation, operation
   state, backups, locking, UI and recovery. Existing task synchronization stays unchanged.
   Cost: vdirsyncer's Python API is not a stable collection-management interface. Detect
   supported versions/API shapes before accessing the network; unsupported installations
   return an actionable error. Cover supported adapters with tests.
2. **Native Rust CalDAV.** Removes that Python API dependency, but duplicates config parsing,
   credential fetching, authentication, TLS and DAV handling. Rejected in favor of the helper.

No separate account configuration or credentials in Twydo. No generic remote URL input in
the list card. Never infer the target collection by display name alone.

## User experience (builder proposals)

- Sidebar `o`: open a list card with Name and Color; select from the existing 2Do palette
  or enter `#RRGGBB`. Require a nonempty name. Choose a fresh internal collection identity.
- Sidebar `i`: edit the selected ordinary list in that card. Preserve its collection
  identity and tasks. Display Archive/Unarchive and Delete actions there.
- Sidebar `dd`: request deletion of the selected ordinary list. A confirmation names the
  list, counts its stored tasks including completed tasks, and explicitly says deletion
  affects CalDAV and other devices. Cancel is the default. Views cannot be deleted.
- `:lists`: chooser including archived/hidden lists, so they can be edited or unarchived.
  Distinguish duplicate names while selecting by stable identity. Existing `:list` navigation
  and `:forget-list` keep their meanings.
- Archive removes the list and its tasks from ordinary views, retaining eligible completed
  tasks in Done under the existing visibility contract. The management chooser retains it.
  Warn before archiving a list with unfinished tasks. This is a local action and stays
  available offline; mark it separately from the card's remote save action. Preserve unknown
  configuration and existing preferences on disk, and make other Twydo processes observe
  the saved preference. Unarchive is available through `:lists`.
  The phone retains tasks in Smart Lists and required Show all in Done for the completed
  sample. Twydo keeps its existing product rule excluding archived lists from its fixed
  Next 3 Days view and its current Done behavior; no new filter is introduced here.
- Long operations run outside the input loop, report progress/failure, and reload on completion.
  A failed save retains entered values. Do not report success for a merely scheduled request.
- List/group management beyond this issue is deferred. Preserve existing group membership
  on edit; any creation placement in an existing group is a builder choice to document.

## Helper boundary and remote identity

Use a bundled helper available to the installed binary, not a repository-relative script.
Pass a versioned structured request through stdin; return structured results without secrets.
Invoke an explicitly configured Python interpreter (default `python3`); verify that its
vdirsyncer dependency matches the supported adapter. A custom sync wrapper does not imply
that its executable is a Python interpreter. Document the helper dependency in installation
and configuration pages.

Resolve the configured pair and the named local filesystem storage. Confirm that its path
matches the bound Twydo vdir and that exactly one opposite storage is CalDAV. Respect
explicit collection mappings; reject unsupported pair layouts rather than guessing.
Resolve a collection through the pair's mapping and discovered identity. Permit only the
specific discovered task collection, never an account root or another collection. Validate
redirects and normalized hrefs before a destructive request. Never create missing unrelated
collections or answer blanket discovery prompts.

For metadata, preserve unknown properties and tasks. Validate DAV multistatus property
results and read back the requested values: HTTP 207 alone is not success. Creation must
produce a VTODO-capable collection with the requested metadata. Refresh discovery only
through the serialized collection operation; reject unresolved prompts.
Name/color editing also requires a demonstrated server-enforced write precondition.
A preflight read followed by PROPPATCH and readback can overwrite an intervening phone
edit; these checks alone must not enable the operation, including in test mode.

## Operation state, sync and recovery

Collection changes are distinct from task operations. Persist inspectable operation records
outside the vdir, with stable operation IDs, exact target, requested values, preconditions,
backup references and phase. Do not store credentials. Serialize with the same run/vdir
locks and finish existing task recovery before collection mutation. Refuse a conflicting
collection operation, unresolved task recovery, queued/pending edits or conflicts affecting
the target. Other lists' operations remain intact.

Collection creation, metadata changes and deletion are explicit remote actions, like
`:sync`, and remain available with automatic sync off. Label the card's action
**Save to CalDAV** and explain its immediate device-wide effect; deletion has its own
explicit confirmation. Never trigger them from background sync while off. Network failure
keeps the card input and a recoverable operation record; no optimistic success. A new list
becomes available for task creation only after its remote collection and local discovery
are confirmed. Do not introduce an offline collection queue in this issue. A local-only
archive action, if that is the evidence-backed implementation, remains available offline.

Deletion needs a fresh remote inventory as well as local data. Before presenting final
confirmation, preserve the exact remote resources (including server-only resources), raw
local files, collection properties and relevant task journal/conflict state in a durable
private backup. Record a manifest of hrefs, ETags and hashes. Refuse unreadable/partial
inventories or unsafe paths; do not back up only what the TUI currently displays.

Before deletion, revalidate the confirmed inventory and local bytes. Changed resources
require a new preview/confirmation. Use a server-enforced collection precondition only if
its protection has been demonstrated; an item ETag or sync token is not automatically a
collection deletion guard. A fresh inventory followed by DELETE still has a remote-writer
race. If the server cannot enforce the needed precondition, do not claim conflict-safe
deletion: the device/implementation checkpoint must resolve that limitation before enabling
the feature. Never silently drop remote edits made after the backup.

Persist the backup before any DELETE. Record uncertain outcomes after timeout/interruption;
reconcile the exact collection on retry, never repeat a create/delete against a newly
resolved name. Only after confirmed remote absence retire the local copy and refresh
discovery so normal sync cannot recreate it. Interrupted retirement is recoverable before
normal task sync resumes. Distinguish already-absent targets from unreachable servers.

Recovery must be operable through Twydo, not only a backup path: browse the preserved list,
restore into a newly created or explicitly selected destination, and use guarded writes.
Restore remote-only content too. Preserve the source backup; do not overwrite a recreated
collection, replay stale journal work, or describe recovery as an ordinary task undo.
Retain unknown raw resources even if the existing task-gated recovery path cannot import
them; explain that limitation explicitly and provide an export path. No automatic cleanup.

## Bounded archive investigation

This section is a protocol; the separately recorded
[archive-round approval](../../reference/evidence.md)
authorizes only that bounded session, not general NAS or phone access. Bind actual account, exact
collection identity, transport/config, device versions and private backup/capture paths
outside git before requesting fresh approval for network/device actions.

Prefer one new synthetic-only list `Twydo 69 Archive` on the phone's already configured
account. Do not switch accounts. If a safely isolated account is unavailable, say so and
request explicit permission for this synthetic list on the current account. Do not reuse
or remove existing test lists. A fresh list and two tasks (one open, one completed) are
enough; no recurrence or parent-family matrix is needed.

1. Confirm restorable phone backup and approved account/list scope. Ask for approval for
   the exact setup, captures and phone actions. Create only the approved synthetic list and
   two tasks; if uploading seed fixtures, prepare and allowlist the exact UIDs/hrefs first.
2. Capture a baseline after phone sync: exact collection properties (allprop plus explicitly
   requested known/vendor properties), collection existence and inventory, task ETags and
   bytes. Keep raw responses private. Record list/All/Today/Done visibility on the phone;
   include an eligible synthetic Today task if Today visibility is being checked.
3. Ask the user to archive only this list, then sync. Read the same exact collection and
   resources, compare stable properties and raw task bytes, and ask where the archived list
   and both tasks appear. If the collection disappears or content changes unexpectedly,
   stop; do not recreate or restore automatically.
4. Ask the user to unarchive only this list, sync and repeat the same comparisons. Confirm
   restoration of visibility, identity, tasks, completion state and metadata.
5. If a reversible shared property emerges, propose a separate helper-written toggle plus
   phone follow-up to prove Twydo → phone behavior. No server write is authorized by a
   successful phone-only observation. If no shared change is observed, report only that
   result; one device does not establish cross-device local preference behavior.

Give one phone action at a time and capture before continuing. Stop for stale identities,
unrelated writes, conflicts or unexpected resources. No account/global write-mode changes,
normal background sync, list deletion, cleanup or production task inspection in this round.
Only sanitized synthetic fixtures and findings enter git. Exact account identifiers, URLs,
credentials, raw multistatus responses and backups stay outside the repository.

## Verification and release boundaries

Implement in bounded plans after this design is approved. Start with the archive evidence
and helper/collection safety primitives; UI work builds on those contracts. Creation,
metadata, archive and deletion have separate evidence/capability verdicts. Do not enable a
remote operation just because task writes are allowed or another collection operation passed.

Use synthetic local DAV responses for identity/path, authentication adapter, multistatus,
partial failure, unknown metadata, remote-only resources, concurrency, crash/retry, backup,
recovery and sync-off tests. Exercise real TUI keys, cancel paths and duplicate names.
Representative device rounds must establish create/name/color propagation, any shared
archive representation, deletion and recovery; prepare their bounded scopes separately.

Update as-is docs in the same implementation changes. Required checks: appropriate Rust
and helper tests, workspace tests/lint/format, Python tool tests, documentation links and
whitespace/privacy review. No release, push, installed-binary replacement or production
device round is implied by this design.

## Open decisions and builder latitude

- Archive remains local under the observed evidence; a shared implementation would need
  a separate representation and interoperability proof.
- Resolve server-enforced deletion concurrency protection before shipping deletion.
- Builders may choose module layout, adapter packaging, structured error fields, operation
  schema and card geometry. Reuse the current picker, palette, locks and recovery UI where
  their contracts fit; do not weaken existing task gating or preservation.
