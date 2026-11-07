# Issue 41: Twydo-written item ordering

Status: phone display/edit-return checks completed for both ordering operations with
**Manually** selected in 2Do. The user rejected the initial simple-family-only gate; the
correction treats ordering as a field-specific operation across recurring/date/status
contexts while preserving those fields and retaining the independent creation gate.
Direct phone samples remain plain families; existing recurring captures support exact
preservation/queue/undo/completion regressions. No additional device round was performed.

## Contract and existing evidence

Phone-side reordering is unavailable, as established in the
[issue 35 session](issue35-checklist-order.md) and confirmed by the user. Do not repeat drag
attempts or require an iPhone-originated reorder. Those captures establish the shared
`DisplayOrder` representation for append, not its precedence over creation time.

The new checks establish that the phone respects an order Twydo writes and preserves it
through an unrelated phone edit. An unsupported phone baseline is therefore expected for
these Twydo-originated operations; it is not evidence that the round trips passed. Other
operations retain their existing baseline requirements.

## Bounded round — completed; do not repeat

The following is the completed protocol, retained for provenance. Further rounds require
fresh approval; the results below do not authorize repeating any action.

Use only synthetic, undated, non-recurring checklist and project families in the existing
approved test collection. Proposed new families are `Order 41 Checklist` and `Order 41
Project`, each with Zulu, Alpha, Mike created in that order. Prefer seeding these through
`tools/seed_fixture.py`; do not type a new family on the phone or repeat append research.
Initial scope is two parents and six children. Anchored insertion adds four children, two
per parent. No dates, alerts, completions, recurrence, deletion, cleanup or account changes.

Before requesting approval for any access, bind the collection, family/resource UIDs, device
versions, private candidate/capture paths and exact sync method outside git. Identify whether
new-family seeding is needed and include its eight create-only resources in the approval.
Do not reuse or modify other synthetic families without explicitly agreed scope. Prepare
local candidates first; validate that the request can be confined to the agreed resources.
If existing private session state cannot supply these bounds without new access, request
that access separately. Never inspect production task collections for this purpose.

Give one phone action at a time, capture and compare before proceeding:

1. **Initial state:** establish the two synthetic families and capture their baseline.
   Record creation times/UIDs and confirm Zulu, Alpha, Mike. This is setup for the reorder
   test, not a repeat of the append test.
2. **Twydo reorder:** write Mike before Zulu for each parent. Upload only changed sibling
   resources after fresh dependency reads, with update preconditions. The parent and any
   unchanged sibling must retain their bytes. Ask the user to sync and confirm Mike, Zulu,
   Alpha on the phone. Creation timestamps must remain unchanged.
3. **Phone preservation:** ask the user to rename Mike to MIKE in each parent and sync.
   Capture only changed resources. Verify UID, links, all ranks, creation times and unrelated
   fields; Twydo and the phone must still show MIKE, Zulu, Alpha.
4. **Anchored insertion:** Twydo inserts `Before` immediately before Zulu and `After`
   immediately after Alpha in each parent, in separate journal operations. Upload only the
   four new resources and any sibling resources whose rank changes, after guarded fresh
   reads. Ask the user to sync and confirm MIKE, Before, Zulu, Alpha, After in both parents.
5. **Insertion preservation:** ask the user to rename Before/After to BEFORE/AFTER and sync.
   Capture only changed resources; verify placement and identity/rank preservation on return.

Stop on stale dependencies, conflicts, unrelated changes, unexpected resources or a display
mismatch. Do not broaden the operation, retry by overwriting, change global write mode,
switch accounts or delete test families. Record a mismatch as a failed/unverified contract.

Store only changed/new synthetic evidence in git, referencing unchanged captures. Verify
both directions of this contract separately for checklist and project parents. Mark only
the exact undated, non-recurring rows supported by successful captures. Recurring families,
dated families and independent project-child recurrence remain outside this round.

## Local verification

The implementation is exercised against synthetic captured families by
`crates/twydo-core/tests/store_order.rs` and `crates/tdo/tests/item_order.rs`, covering
reordering/insertion, queue overlays and drain, stale dependencies, durable recovery,
undo/redo, hidden siblings, context restrictions and Tasks/Details consistency. These tests
prove local behavior; they do not replace the phone display/preservation round.

## Approved read-only preflight (2026-11-06)

The user approved inspecting the existing private test-session setup and read-only checks
of the synthetic test collection. Reused the existing credential helper without printing
credentials or changing configuration. One collection-identity PROPFIND, two synthetic
parent-title-filtered REPORTs and eight candidate-path GETs confirmed the expected test
collection, no proposed parent-title collisions and eight absent candidate paths. No
unfiltered task query, upload, sync run, phone action or cleanup was performed.

Eight baseline candidates were prepared offline with `tools/seed_fixture.py`, fresh UIDs
and remapped links, retaining source creation timestamps to distinguish insertion order
from display ranks. Local `tdo export-json` reads all eight. Exact resource identities,
hashes, collection binding and the initial create-only upload scope are private, outside
git. The next approval is for creating these eight baseline resources only; reordering
existing resources will be prepared and presented separately before upload.

## Approved baseline creation (2026-11-06)

The user approved creating exactly two new synthetic parents and six children. A fresh
repeat of the read-only preflight passed before uploading. Exactly eight PUTs with
`If-None-Match: *` returned HTTP 201; each resource passed immediate byte-for-byte server
read-back. No existing resource, normal vdir, sync state, account or configuration changed.
The new resources are in [000-seeded](../../../fixtures/phase0/v1/issue41-item-reorder/000-seeded/).
Exact target binding, request receipts and hashes remain private. Do not repeat these creates.

The user has been asked to sync the phone and inspect both baseline sequences as Zulu,
Alpha, Mike, without editing. Confirmation is pending. In an isolated local copy with no
sync configured, the actual Twydo TUI moved Mike up twice under each parent using `K`.
The six candidate child resources now have Mike=0, Zulu=1, Alpha=2; both parent resources
are byte-unchanged, as are creation times and identities/links. This reorder candidate
remains private and is not uploaded. Present its exact six-resource scope for fresh approval
after baseline confirmation and fresh dependency checks.

## Phone baseline display mismatch (2026-11-06)

The user confirmed checklist display **Zulu, Alpha, Mike**, but project display **Alpha,
Mike, Zulu**. Reorder uploads were held at this mismatch. A read-only capture of exactly
the eight known synthetic resources found every server resource byte-unchanged from
`000-seeded`, with all item `DisplayOrder` values still numeric zero and no shared sort
field added. [010-phone-baseline](../../../fixtures/phase0/v1/issue41-item-reorder/010-phone-baseline/)
therefore stores only the step record and references all eight unchanged resources.

The project’s alphabetical display could reflect a phone-local per-project sort preference
or a different tie-break for zero positions; neither explanation is established yet. The
user was asked to inspect the project’s sort setting without changing it. This baseline
does not establish whether distinct Twydo-written ranks override the current project order.
No reordered resources were uploaded, no sort settings changed, and all reorder/insertion
round-trip matrix verdicts remain unverified. Do not repeat baseline creation or request
phone-side reordering.

### Project sort setting

The user reported **Status** for `Order 41 Project`. All baseline items are open, and their
shared positions are tied at zero. Alphabetical display within that status group is observed;
it does not yet establish how distinct Twydo-written ranks are treated. Leave Status selected.
The next proposed check is the prepared six-child update to Mike=0, Zulu=1, Alpha=2 under
both parents, using fresh dependency reads and per-resource ETag preconditions. The local
candidate differs only in `DisplayOrder` and normal modification timestamps; parents,
creation times, identities/links, status and other fields are preserved. Fresh upload
approval is still required. No phone sort change is requested or authorized.

## Approved Twydo reorder upload (2026-11-06)

The user approved exactly six existing synthetic child updates to Mike=0, Zulu=1, Alpha=2,
leaving project Status sort selected. The first fresh dependency check stopped before any
write: only the project parent’s modification timestamps had changed since the earlier
return. [015-parent-stamp](../../../fixtures/phase0/v1/issue41-item-reorder/015-parent-stamp/)
stores that one changed resource and references seven unchanged resources. A fresh local
copy preserved the newer parent and regenerated the same six-child candidate through the
actual Twydo TUI (`K` twice on Mike in each family), without configuring sync or changing
global write mode. All logical fields other than positions, plus normal modification
timestamps, were checked unchanged.

The subsequent preflight matched both complete families and all eight dependency hashes.
Exactly six `If-Match` PUTs returned HTTP 204; membership and dependency bytes were checked
before each update and all written resources passed exact server read-back. Both parent
resources retain their latest bytes. No creation, deletion, unrelated-family access,
account/configuration change or phone sort change was performed.
[020-twydo-reordered](../../../fixtures/phase0/v1/issue41-item-reorder/020-twydo-reordered/)
stores only the six changed children and references the two unchanged parents. Do not repeat
these writes. Phone display inspection has been requested and is pending; no rename or
insertion has been requested yet. Compatibility verdicts remain unverified.

## Phone display return after reordered ranks (2026-11-06)

After sync, the user reported that one or both displayed sequences differ from the expected
Mike, Zulu, Alpha. The exact checklist/project sequences were requested separately; do not
assume which family differs. Rename and insertion steps are on hold. A scoped read-only
return found all eight resources byte-unchanged from the successful upload, preserving
Mike=0, Zulu=1, Alpha=2 for both families.
[030-phone-display](../../../fixtures/phase0/v1/issue41-item-reorder/030-phone-display/)
stores only a step record referencing all unchanged resources. This shows rank preservation
through sync, but does not establish correct phone display or preservation through a phone
edit. No compatibility verdict was enabled, no phone setting changed and no further write
was performed. Await the exact displayed sequences before proposing another bounded check.

## Manually display and rename preservation (2026-11-06)

The user selected **Manually** and confirmed the saved order now appears correctly. This
resolves the prior display mismatch: shared ranks were preserved under Status, but that
phone sort did not display the intended sequence. No shared sort field appeared after the
change; only parent modification timestamps changed.
[040-manual-display](../../../fixtures/phase0/v1/issue41-item-reorder/040-manual-display/)
stores the two changed parents and references the six unchanged child resources.

The user then renamed Mike to MIKE under each parent, synced and confirmed retained
MIKE, Zulu, Alpha display. The scoped return verified all identities, parent links, creation
times, dates, statuses, recurrence, complete JSON metadata/ranks and unrelated properties.
Only the two requested summaries and normal modification timestamps changed; one parent
received a timestamp change too.
[050-phone-rename](../../../fixtures/phase0/v1/issue41-item-reorder/050-phone-rename/)
stores three changed resources and references the five unchanged resources.

The plain checklist/project `reorder` rows now pass both Twydo directions, with the
unavailable phone-originated reorder baseline still marked unsupported. Dated, recurring
and completed/cancelled contexts are not enabled by this evidence. Phone Manually selection
is a documented display prerequisite; no Twydo sorting controls or phone-setting writes
are added. The `insert` rows remain unverified. Four new Before/After resources plus four
Zulu/Alpha rank updates were prepared locally through real `o`/`O`; parents and MIKE
resources remain byte-unchanged. No insertion upload is authorized.

## Approved anchored insertion upload (2026-11-06)

The user approved the exact next scope: four new Before/After resources and four existing
Zulu/Alpha rank updates across the same two synthetic parents. Both parent resources and
both MIKE resources remain byte-unchanged. Candidates came from actual `O` before Zulu
and `o` after Alpha in isolated local copies of the phone rename return, without sync or
global mode/configuration changes. Expected sequence is MIKE, Before, Zulu, Alpha, After.

A fresh preflight matched both families and all eight dependencies and confirmed all four
new paths absent. Exactly four create-only PUTs returned 201 and four ETag-guarded updates
returned 204, with fresh membership/hash checks before each write and exact read-backs.
[060-twydo-inserted](../../../fixtures/phase0/v1/issue41-item-reorder/060-twydo-inserted/)
stores only the eight new/changed resources, referencing the four unchanged parents/MIKE
resources. Do not repeat these writes. Phone display confirmation was requested with
Manually unchanged; no Before/After rename has been requested yet. `insert` rows remain
unverified until display and phone-edit-return checks pass.

## Anchored insertion display confirmed (2026-11-06)

The user confirmed both parents display MIKE, Before, Zulu, Alpha, After after sync with
Manually selected. A scoped read-only capture verified all twelve resources byte-unchanged
from `060`, with complete metadata/ranks/identities/links and unrelated fields preserved.
[070-phone-insertion-display](../../../fixtures/phase0/v1/issue41-item-reorder/070-phone-insertion-display/)
stores only a step record and references all twelve unchanged resources. The user has been
asked to rename only Before/After to BEFORE/AFTER under both parents, then sync and confirm
retained order. That preservation return is pending; `insert` remains unverified.

## Anchored insertion rename return passed (2026-11-06)

The user renamed Before/After to BEFORE/AFTER under both parents, synced and confirmed
retained MIKE, BEFORE, Zulu, Alpha, AFTER display. All twelve resources were captured
read-only. The four renamed resources preserved their UIDs, standard/JSON parent links,
creation times, all `DisplayOrder` ranks, complete JSON metadata and unrelated task fields.
The other eight resources remained byte-unchanged.
[080-phone-insertion-rename](../../../fixtures/phase0/v1/issue41-item-reorder/080-phone-insertion-rename/)
stores only the four changed resources and references the eight unchanged resources.

The strict initial property comparison flagged four additional changes on each new item.
These match the known phone initialization of Twydo-created tasks, already captured in
[Plan 7](plan7-capture.md) and the [issue 35 return](issue35-checklist-order.md):

| Property | Twydo-created value | Phone return |
|---|---|---|
| `PRODID` | `-//Twydo//tdo 0.1.4//EN` | `-//2DOAPP.COM//2Do 3.8//EN` |
| `X-2DOAPP-SYNC-IMAGE` | absent | `1` |
| `X-2DOAPP-SYNC-AUDIO` | absent | `1` |
| `X-2DOAPP-AUDIO-DURATION` | absent | `0.000000` |

Verification then checked those exact transitions and every remaining field, rather than
ignoring all property differences. No meaning beyond this observed representation is
assigned to the sync/audio values. The `insert` checklist/project rows now pass both
Twydo directions with the unavailable phone-originated reorder baseline still unsupported.
The capture regression reads all twelve tasks, verifies uppercase renamed positions and
unchanged metadata/semantics, and exercises a later Twydo reorder/undo while preserving
the phone-added properties. No production task data or account settings were changed.
Do not repeat any upload or delete the synthetic families. No additional device round is
authorized by this completed session.

## Correction: recurrence is not an ordering capability (2026-11-07)

The initial gate required separate recurring, scheduled and closed-member ordering rows.
The user rejected that restriction: their checklists recur, and pure ordering has the
same field representation. Inspection confirmed that the reorder writer touches only
DisplayOrder and modification timestamps, with parent/sibling holds and journal guards.
No recurrence parsing, date arithmetic, status mutation or recurrence execution occurs.

The corrected gate checks the verified ordering operation by declared parent kind and
validates editable JSON, identity, links, collection and non-nested family structure. It
reuses the observed rank representation across recurrence/date/status contexts instead of
requiring unrelated capability verdicts. Anchored insertion still passes the existing
complete Create gate, including inherited-date/recurrence restrictions. Recurrence/edit/
completion rows and global write mode are unchanged.

Direct new reorder phone captures remain the plain-family samples above. The extension
is justified by exact preservation tests over existing Plan 8 recurring checklist and
Plan 6 recurring/project-child captures, preserving every property/component/parameter
except the intended rank/stamps. Tests include queue/undo, mixed completion states, dates,
alarms and opaque/unverified recurrence fields, the normal verified TUI, and retention
of new ranks through an already-supported recurring checklist completion and its undo.
This does not claim any new phone round or newly enabled recurrence execution. No NAS,
iPhone, production task or account/configuration access was performed for this correction.
