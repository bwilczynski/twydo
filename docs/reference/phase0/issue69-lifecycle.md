# Issue 69: collection lifecycle evidence

Status: helper creation confirmed on the phone; both tested guard mechanisms failed.
The disposable list was removed by the approved negative DELETE test. All three
captured tasks remain intact in private backup and verified local export.
Protocol: [bounded lifecycle round](issue69-lifecycle-protocol.md).

## Approval — 2026-11-09

After completing the history squash, draft PR and metadata safety-gate fix, the user
explicitly approved the prepared NAS/iPhone round with **yes**. This permits only the
new lifecycle/recovery list identities and three allowlisted synthetic tasks described
in the protocol. Account binding, connection settings, exact URLs, credentials, raw
responses and backups remain outside git. No other lists or tasks are in scope.

## Helper creation and two-task baseline

- The prepared Rust service driver created **Twydo 69 Lifecycle**, color `#3289D2`,
  through the bundled vdirsyncer helper. Remote creation, VTODO support, metadata
  readback, local finalization and discovery refresh completed successfully.
- A read-only service deletion preview captured the empty collection and its properties
  in a durable private backup. It returned zero remote resources and two local metadata
  files. No DELETE was sent.
- `tools/seed_fixture.py` prepared exactly **Lifecycle 69 Open** and **Lifecycle 69 Done**
  in the isolated local vdir. A sync of only that collection uploaded both. No metasync
  or sync against the user's normal local vdir was run.
- Before the first phone check, a fixture correction added required `DTSTAMP` fields
  ([RFC 5545 §3.6.2](https://www.rfc-editor.org/rfc/inline-errata/rfc5545.html)).
  Vdirsyncer intentionally ignores timestamp-only differences in its content hash,
  so ordinary sync did not upload that correction. Its resource-update API then applied
  the exact two corrected resources with their captured strong `If-Match` ETags.
  Earlier raw bytes and receipts remain in the private backups.
- The final read-only preview captured exactly two resources. Both match the sanitized
  [baseline fixtures](../../../fixtures/phase0/v1/issue69-lifecycle/000-helper-baseline/step.toml)
  byte for byte, including UID, title, status and stamp. The completed sample is undated,
  with synthetic completion time `2026-11-08T12:00:00Z` (13:00 Europe/Warsaw).
- At this baseline stage, **Lifecycle 69 Remote Only** was not yet uploaded and no
  guard test had run. Later steps below record those separate observations.

## Confirmed phone baseline

The user confirmed the blue lifecycle list, its open task and its completed task after
sync. Done → Show all was permitted. A fresh server capture still matched both task
resources byte for byte. This establishes only the create direction from Twydo to the
phone; reverse creation remains unverified.

## Exclusive-lock candidate — failed metadata enforcement

The server granted an exclusive depth-infinity write lock with a 60-second timeout
and returned a lock token. It omitted lockroot and returned property 404 for
lockdiscovery even while the lock was active. Read-only validation probes released
their locks with HTTP 204; no content mutation was used to validate those grants.

A separate approved negative probe bound the exact request/response URL and unique
owner/token, preserved both task resources, then attempted a same-account name-only
PROPPATCH **without** the token. HTTP 207/property success and readback confirmed
the name became **Twydo 69 Lifecycle Edited**, retaining the blue color. The lock
was released with HTTP 204. No further lock-based writes were attempted.

This disproves that lock's protection for metadata edits; it does not establish
protection for other write methods. Metadata editing remains refused in every mode.

## Calendar-export ETag candidate — failed conditional deletion

Read-only HEAD/GET exposed a strong calendar-export ETag, despite collection getetag
being unavailable through PROPFIND. It remained unchanged by the name edit. Its
resource coverage was tested separately within the approved version-check scope:

1. The fixed **Lifecycle 69 Remote Only** task was uploaded create-only. The export
   contained all three known UIDs and its ETag changed. No local copy of that task
   was placed in the active isolated vdir.
2. An `If-Match` update advanced only that task's DTSTAMP by one second. The export
   ETag changed again; its UID, title and lifecycle remained unchanged.
3. The service published a durable backup containing all three exact remote resources,
   four raw local files and relevant state. The remote-only task was verified in it.
4. One approved negative DELETE submitted the obsolete pre-addition ETag in `If-Match`.
   The server returned **204**. Subsequent PROPFIND and GET at the saved exact URL
   reported not found; no newly resolved name or replacement collection was used.
5. The user synced the phone and confirmed the disposable list was gone.

Changing export ETags therefore do **not** demonstrate an enforced collection DELETE
precondition on this server. No fresh unconditional DELETE or retry was sent.
Sanitized task snapshots and result summaries are in
[guard evidence](../../../fixtures/phase0/v1/issue69-lifecycle/020-guard-evidence/step.toml).

## Recovery evidence and final scope

The existing core backup-export API exported all three remote resources locally;
each SHA-256 matched the captured resource. It also retained the two local task files,
and the original backup remained unchanged. This verifies snapshot recovery/export,
including the server-only resource. It does not verify a remote collection restore.

Only the isolated lifecycle vdirsyncer configuration was sealed to prevent ordinary
sync from recreating this removed test target. The user's normal configuration, account,
archive test list and older synthetic families were not changed. No **Twydo 69 Recovery**
collection was created: the protocol's prerequisite of passing deletion protection failed.
No automatic cleanup was performed.

The matrix records independent failed atomicity edges for metadata/deletion. The
metadata service now requires atomicity evidence independently of all phone verdicts,
in both write modes; the existing product DELETE refusal remains in force. A regression
proves that promoted interoperability columns cannot bypass a missing, unverified,
failed or unsupported metadata-atomicity verdict. No safe-write permission is inferred
from the successful backup or from the phone observing deletion.

Further metadata/deletion implementation needs a proven server mechanism or an explicit
policy decision about best-effort writes and changes arriving after the last snapshot.
