# Issue 69: collection management evidence

Status: approved archive/unarchive round captured; no shared CalDAV representation observed.

Design: [list management](../../superpowers/specs/2026-11-08-issue69-list-management-design.md).
The user selected the vdirsyncer helper and requested device verification of archiving.
Deletion from CalDAV/all devices is in scope for implementation, with confirmation and
recovery; that does not authorize deleting any existing collection during this session.

## Existing evidence

The [2Do reference](../2do/01-features.md#10-lists-list-groups-sections-inbox) describes
archive visibility but supplies no CalDAV archive encoding. Twydo currently uses the
documented local `ListPrefs.archived` fallback. Earlier task/family phone rounds do not
verify archive/unarchive or authorize repeating their writes.

The user approved local inspection of existing private setup on 2026-11-08. The current
configuration has a filesystem/CalDAV pair, credential fetching and name/color metasync.
Previous synthetic sessions used the normal account connection. The phone's current
account and a restorable backup have not yet been confirmed. No credential-fetch command,
network request, sync, phone operation or user-configuration mutation was performed.

## Approved bounded archive round (completed)

Use a single **new** list named `Twydo 69 Archive`, containing two synthetic plain tasks:
`Archive 69 Open` (due on the local test day, no alarm) and `Archive 69 Done` (completed,
undated). No recurrence, parents or real obligations. Existing synthetic lists stay intact.

- [x] User confirms the phone is already using the intended account and has a restorable
  backup. Never change account/provider as part of this round.
- [x] Obtain explicit approval for this new list, its two tasks, phone archive/unarchive
  actions and bounded server reads. If using the normal account, say that explicitly.
- [x] Bind the exact collection href/identity and two task UIDs privately. A read-only
  collection-property listing may identify the newly created list; do not read task content
  from any other collection. Stop if the proposed name is already present or ambiguous.
- [x] Prepare exact seed resources offline with `tools/seed_fixture.py` where possible;
  allowlist their identities before any create-only PUT. Separate approval of exact server
  writes is required; manual phone creation can avoid this server-write step entirely.
- [x] Baseline: after phone sync, capture only this collection's properties, resource
  inventory, ETags and raw resources. Record list/All/Today/Done visibility on the phone.
- [x] Phone action 1: archive only `Twydo 69 Archive`, sync, and report visibility of the
  list, its open task and completed task. Capture the same server scope before continuing.
- [x] Compare collection presence/properties and task bytes. Stop on disappearance,
  unexpected mutation, unrelated content or conflicts; do not recreate anything.
- [x] Phone action 2: unarchive only that list and sync. User confirmed the action;
  server capture verified identity, lifecycle and metadata. A separate per-view phone
  visibility confirmation after unarchive was not supplied; do not claim one.
- [x] Record observed changes and limits. A phone-originated archive flag does not yet
  authorize helper-originated writes; any such verification needs a fresh bounded scope.

Keep raw collection responses, URLs/account identifiers, backups, credentials, transport
logs and exact session manifests outside git. Commit only sanitized synthetic evidence.
No normal-pair sync, global settings change, list deletion, cleanup or other data access.
Give one phone action at a time and wait for its capture/comparison before the next.

## Offline preparation (2026-11-08)

Prepared exactly two resources from the existing plain-task Plan 4 capture using
`tools/seed_fixture.py`, with fresh identities and synthetic titles. In an isolated local
vdir/config with no sync configured, the current core CLI set the open task due today,
removed both tasks' start dates, cleared the other task's due date and completed it.
`tdo export-json` parsed the resulting candidates successfully. Their exact identities,
hashes and private candidate directory are recorded outside git.

Documentation links and whitespace checks passed for the draft design and protocol.

## Approval (2026-11-08)

The user approved the presented archive-only round on the phone's current normal CalDAV
account, including confirmation of the account and restorable backup: read collection
properties to check name availability; create one new synthetic `Twydo 69 Archive` list;
seed exactly the two prepared tasks; capture that list's properties/tasks; then archive
and unarchive it on the phone one action at a time. No account change, normal-pair sync,
other task reads, collection deletion or cleanup. The exact prepared resource identities
and hashes were linked in the approval request and remain private. The user subsequently
requested focused commits. This approval does not approve the whole draft product design
or a helper-originated archive write.

## Read-only name preflight (2026-11-08)

One Depth-1 PROPFIND of collection names/types confirmed the known synthetic account
collection and that `Twydo 69 Archive` is absent. No task content was requested and no
server write, sync or configuration change occurred. Raw properties and the request
receipt remain private.

The configured DAV entry point and cached canonical collection home differ in endpoint
prefix. Before any credential fetch/request, verified HTTPS/same origin, the same account
path component, the known synthetic collection/local identity, and that every cached
remote collection shares the canonical home. The initial stricter prefix assertion stopped
before network access; the validated canonical mapping then passed the bounded request.

At that checkpoint, the next phone action was to create only the new empty list and sync. Resolve and
allowlist its exact server identity before seeding the two approved resources. Do not
archive until the baseline is captured and both tasks are visible on the phone.

## List creation and seeding (2026-11-08)

The user confirmed creating/syncing the new empty list on the phone. Collection-property
discovery uniquely resolved that exact name, verified VTODO support and an empty inventory,
and bound its identity privately. Captured all returned collection properties plus explicitly
requested DAV/CalDAV name/type/description/color/token fields before seeding.

Exactly two create-only PUTs with `If-None-Match: *` returned HTTP 201. The first raw
read-back comparison stopped the sequence before the second upload: the server converts
LF to CRLF. A guarded GET proved that only line endings differ, with equal unfolded
content and unchanged property names. Rechecked the one-resource membership, candidate
hashes and second-path absence, then created only the remaining approved task. Both
read-backs equal the approved candidates after LF/CRLF normalization only. No existing
resource was overwritten, retried or deleted.

The seeded capture contains exactly the two expected resources and 14 returned collection
properties. Compared with the empty baseline, only sync-token, getctag and getlastmodified
changed among those properties. Raw properties, exact identity, task bytes, ETags and
request receipts remain private. The next step is phone sync and baseline visibility
confirmation before asking the user to archive.

## Phone baseline (2026-11-08)

The user confirmed `Archive 69 Open` appears in the new list, All and Today, and
`Archive 69 Done` appears in Done after phone sync. The post-sync capture is byte-identical
to the seeded server state: both task bodies and ETags and all 14 returned collection
properties are unchanged. Archive/unarchive effects were untested at that checkpoint.

## Phone archive (2026-11-08)

The user reported that 2Do asks for confirmation because the list still contains unfinished
tasks, explaining that archived lists/tasks stay hidden except in Smart Lists and Done.
After archive/sync, the user could still see the test content in the Next 3 Days smart list.
The open task was no longer reported in the ordinary views. The user subsequently confirmed
the completed task appears in Done after selecting **Show all**, but is hidden by Done's
default filter. The Smart Lists/Done exception therefore does not mean unconditional
visibility under every phone filter. Do not infer the filter's implementation or a shared
encoding from this observation.
The completed candidate has no due date and was completed on the local test day.

The archived capture still contains the exact same two task bodies/ETags and all 14
returned collection properties as the phone baseline. No shared archive representation
was observed in these responses. The property set includes name, color, calendar-order,
supported component types and the returned DAV creation/modification/content/owner/token
properties. Raw responses and account-bearing values stay private. This is evidence of
an unchanged observed CalDAV state, not proof that no other shared representation exists.

Twydo's fixed Next 3 Days view currently excludes archived lists by product decision;
2Do's observed Smart List exception differs. Keep that distinction visible in the design
review rather than silently changing Twydo's view rules. A helper-originated archive write
is not justified by this phone-originated observation.

The user could not locate unarchive on the iPhone. No additional server requests/writes
were made in response. The current official
[iOS list documentation](https://www.2doapp.com/docs/ios/lists-and-list-groups/)
places it under Settings → Security → Archived, with an Unarchive action on the list.
That was documentation guidance, not an observed path on the user's installed version.

## Phone unarchive and result (2026-11-08)

The user found the actual path on their installed iPhone app:
**Settings → Privacy → Archived Lists**. They unarchived `Twydo 69 Archive` and synced.
The final capture again has unchanged task bytes, resource ETags and all 14 returned
collection properties. It equals both the archived capture and the original seeded/phone
baseline. No collection or task was deleted. Keep this new synthetic list and its two tasks;
the round did not authorize cleanup or another device experiment.

Synthetic evidence: [seeded baseline](../../../fixtures/phase0/v1/issue69-archive/000-seeded/),
[phone baseline](../../../fixtures/phase0/v1/issue69-archive/010-phone-baseline/),
[archived](../../../fixtures/phase0/v1/issue69-archive/020-phone-archived/) and
[unarchived](../../../fixtures/phase0/v1/issue69-archive/030-phone-unarchived/).
Only the two explicitly approved synthetic resources and sanitized observations are
included. Unchanged steps reference the seeded bytes; raw DAV responses, account-bearing
property values, ETags, URLs and private backups remain outside git.

Conclusion: no archive representation was observed in the captured CalDAV collection
properties or resources. Retain explicitly local Twydo archiving; do not claim that the
phone and Twydo share archive state. This single-device observation does not establish
whether 2Do uses an additional private settings-sync channel. The phone's Smart List and
Done-filter exceptions are recorded above; Twydo's fixed-view behavior stays a separate
product decision. No compatibility verdict for helper-created lists, metadata edits,
collection deletion or recovery follows from this round.

## Implementation checkpoints

An independent builder review found no additional planning questions beyond shared archive
representation and collection-delete concurrency protection. The archive round addresses
the former. Collection deletion/recovery verification will be proposed separately and must
not use this archive-only approval.
