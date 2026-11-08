# Issue 69: proposed collection lifecycle round

Status: **approved 2026-11-09; stopped after failed guard tests**. The phone confirmed
the helper-created baseline, then disappearance after the server ignored stale
conditional deletion. All three resources remain backed up/exported; no recovery list
was created. See the [session results](issue69-lifecycle.md). The completed
[archive round](issue69-list-management.md) does not authorize this round.
The [design](../../superpowers/specs/2026-11-08-issue69-list-management-design.md)
requires a separate collection deletion guard; task ETags and a collection sync token
alone do not establish that guard.

## Exact proposed scope

Use the phone's already configured account, with credentials through the existing
vdirsyncer settings/helper. Private isolated configurations, local vdirs, state and
backups are prepared outside git. No account, normal user configuration, installed
binary or global write-mode changes. No ordinary sync against the normal local vdir.

- One new list **Twydo 69 Lifecycle**, initial color `#3289D2`.
- Helper edit: **Twydo 69 Lifecycle Edited**, color `#379417`.
- Phone edit: **Twydo 69 Lifecycle Phone**, color `#FF8B00`.
- Three allowlisted synthetic tasks: **Lifecycle 69 Open**, **Lifecycle 69 Done**,
  **Lifecycle 69 Remote Only**. All are undated, without recurrence, parents or alarms.
  The done task has synthetic completion time `2026-11-08T12:00:00Z`.
- A second fresh list **Twydo 69 Recovery**, color `#379417`, only if a deletion guard
  passes and recovery can proceed. Recovery uses the three captured synthetic tasks.

Private manifests bind exact collection IDs, UIDs/hrefs and fixture SHA-256 values.
Collection discovery can read collection identities/names as required by vdirsyncer;
do not fetch any other list's task inventory or resources. Never reuse or remove
**Twydo 69 Archive**, older synthetic families, or real task data.

## Bounded sequence after fresh approval

1. Verify the private binding without printing secrets. Create only the lifecycle
   collection with the bundled helper. Confirm its identity/name/color and VTODO
   support; stop if the identity already exists. Seed only the open and done tasks.
2. Ask the user to sync the phone and confirm that exact list, color and two tasks.
   Preserve the baseline raw bytes, ETags and collection properties privately.
3. Before any helper name/color edit, implement and demonstrate a server-enforced
   conditional metadata write. The current prototype remains refused in every mode;
   do not invoke it directly or change the matrix to bypass this gate. If no protection
   exists, skip that write and leave metadata editing gated. Phone-only name/color
   observation may proceed within the approved scope, but cannot promote the capability.
   With proven protection, apply the fixed helper edit, confirm it on the phone, then
   ask for the fixed phone edit and sync. Verify identity and exact task preservation.
4. Read collection validators/properties and response headers with PROPFIND, HEAD
   and OPTIONS as needed. Upload only the allowlisted remote-only task, omit it from
   the isolated local vdir, and capture a complete raw deletion backup. Observe whether
   any proposed collection validator changes with resource content/membership.
5. If there is **no credible server-enforced collection precondition**, stop without
   DELETE. Record the limitation and request a concrete policy decision; no fallback
   to an unguarded inventory-then-DELETE sequence is authorized.
6. Only with a credible candidate: on the exact disposable lifecycle collection,
   attempt a DELETE with a demonstrably stale collection validator after a captured
   allowlisted task mutation. This test can delete the synthetic collection if the
   server ignores the condition; the raw backup must already be durable. If it is
   ignored, stop and report failure. Do not repeat the deletion against a new target.
7. If the stale condition is rejected, verify the fresh condition's semantics, refresh
   the full inventory/backup and explicit preview, then delete that same synthetic
   collection with the fresh guard. Confirm absence and phone propagation. Retire
   only its isolated local copy and verify that ordinary discovery cannot recreate it.
8. If the guard and deletion passed: create only the fresh recovery collection, restore
   the three captured tasks with guarded writes, verify the raw preserved content and
   ask for phone sync/confirmation. Preserve the source backup and recovery list.

Give one phone action at a time. Unexpected identities, extra resources, unrelated
changes, conflicts or authentication/layout differences stop the round. No automatic
cleanup; only the lifecycle collection is authorized for destructive guard tests and
deletion. Approval does not enable shipping capability rows until evidence passes.

## Local checkpoint

The current branch implements local archive, create/edit service and list cards,
read-only deletion previews, durable raw backups and explicit raw export. Create/edit
remain unverified in the shipping matrix; metadata edits are refused even in test mode
because the prototype cannot protect concurrent phone changes. Collection DELETE, collection restore and
acceptance of a vanished pending-create/edit target remain unavailable pending this
checkpoint. Existing retirement/task recovery remains separate.

The earlier archive capture explicitly returned `404` for the collection's `getetag`
property. Sync-token and ctag were returned; their presence does not establish DELETE
protection. This round must verify a usable mechanism or stop for a policy decision.

## Candidate protection: exclusive WebDAV locks

Local inspection of the already captured `supportedlock` property advertises exclusive
write locks. This candidate was tested and failed: the server accepted a name-only
PROPPATCH without a token while reporting an exclusive depth-infinity lock.
[RFC 4918 sections 7.4–7.5 and 9.10.3](https://www.rfc-editor.org/rfc/inline-errata/rfc4918.html)
describe exclusive write locks, token submission through `If`, and depth-infinity
protection of collection members. The server must enforce these rules even when the
second writer uses the same account.

The bounded checklist below was used only on the new lifecycle list. After its first
content-enforcement failure, no further lock-based mutation tests were performed:

1. Capture the exact synthetic resources/properties first. Acquire a short, finite,
   exclusive depth-infinity lock and validate its token, root, depth and granted timeout.
   Refuse unsupported, ambiguous, partial or infinite grants; release any acquired lock.
2. Verify that another same-account request without the token cannot PROPPATCH list
   metadata, update an existing task, add the allowlisted remote-only task, or delete
   the collection. An ignored lock can modify/delete this disposable list; stop at the
   first failure and retain the complete backup. Never probe another collection.
3. Verify a token-bearing metadata update under the lock, then release it. A subsequent
   PROPPATCH/DELETE using the released token must fail its `If` condition even though
   the collection is no longer locked. This is essential for safe lock expiry; do not
   rely on checking a local timer before an unconditional mutation.
4. Only after enforcement passes may an implementation use the lock: acquire it before
   checking expected metadata or revalidating the confirmed deletion inventory, and
   include its live token in the mutating request's `If` header. Changed inventory
   requires a new preview/confirmation. Keep operation receipts private and preserve
   uncertain outcomes; interrupted operations must never blindly reissue mutations.
5. Confirm lock release before asking for any phone action. Do not hold a lock while
   waiting for user confirmation, and do not enable a capability from advertisement
   or offline simulations alone.

This would provide server-enforced serialization rather than an inventory version
token. If the real server cannot enforce both lock ownership and the stale-token
condition, metadata editing and collection deletion remain refused.

## Alternative export-ETag candidate — failed

Read-only HEAD/GET of the same collection returned a strong calendar-export ETag.
It did not change with the metadata rename, so it cannot protect metadata writes.
Within the already approved version-test scope, it did change after adding the fixed
remote-only task and after a one-second DTSTAMP change to that resource. Each resource
write used its own create-only or strong `If-Match` precondition.

After a complete service-managed three-resource backup, one DELETE submitted the
obsolete pre-addition ETag in `If-Match`. The server returned 204, exact-target reads
reported not found, and the user confirmed phone disappearance. The negative test was
explicitly approved to remove this disposable collection if the server ignored its
condition. No further remote mutations, recreation or recovery followed. The isolated
sync configuration was sealed and the complete backup was exported locally with all
three hashes matching.

Both tested guard mechanisms failed. No successful atomicity verdict or permission to
enable metadata editing/deletion follows from this round. Continuing those features
requires another proven server mechanism or an explicit change to the preservation policy.
