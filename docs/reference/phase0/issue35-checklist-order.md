# Issue 35: checklist ordering investigation

Status: bounded baseline capture approved by the user on 2026-11-05; creation and append
captured and phone display confirmed. Drag reorder is unavailable.
User requested focused commits.

The user subsequently approved extending read-only capture to `Project Order 35` in the
same collection and created/synced it. Evidence for that project includes only its own
parent and children; do not copy `Order 35` resources into the project scenario. Future
barriers retain only changed/new resources and reference unchanged earlier captures.

## Question and existing evidence

Issue 35 reports inconsistent placement while adding checklist items. The user also observes
different saved item order in Twydo and the iPhone app. Twydo currently sorts children using
the view's sort and breaks ties by title; it does not interpret `DisplayOrder`. New task
metadata sets that field to zero. All 750 child resource snapshots in the existing phase-0
fixtures have zero there, so they do not establish reorder semantics or direction.

Reuse existing checklist creation and parent-link evidence from
[Plan 6](plan6-baseline.md) and [Plan 7](plan7-capture.md). This investigation does not repeat
recurrence or completion rounds from the [coverage ledger](plan8-tui-completeness-session.md).

## Proposed scope

One new synthetic, non-recurring checklist named `Order 35` in the existing `Twydo test`
list. No dates, alerts, notes, completions, deletions or account changes. The user creates
and changes this checklist on the phone. Agent access is limited to reading its parent and
child resources and a scoped pull/capture of that collection, after validating the sync
target and obtaining permission. No Twydo writes or uploads in this baseline round.

Before access, bind the collection ID, task UIDs, approved private capture paths, device
versions and sync configuration outside git. Check the pull path can be confined to this
collection without uploading pending local edits or bypassing shared recovery/locking.
Stop and obtain agreement if it cannot. Keep raw account/configuration data outside git;
only these synthetic resources may become fixtures. Preserve all test data afterwards.

## Three capture barriers, one phone session

Give one action at a time; capture and compare before proceeding.

1. **Creation:** user creates `Order 35`, adding `Zulu`, `Alpha`, `Mike` in that order,
   then reports the displayed order and current checklist sort setting, if exposed.
   Capture parent and three children, including ordering metadata and timestamps.
2. **Reorder:** if supported, user drags `Mike` ahead of `Zulu`, reports the result and
   syncs. Capture the same UID set and identify every changed property. If reorder is
   unavailable, record the restriction and stop before inventing an equivalent action.
3. **Append:** user adds `Bravo`, reports its position and syncs. Capture the parent and
   four children. Compare displayed order with creation time, title and ordering fields.

Stop on unexpected resources, conflicts or unrelated writes. No additional phone rounds,
sort changes or cleanup are authorized by this checklist.

## Outcome

Document which fields explain the phone's displayed order and whether they survive sync.
Use the captures to propose the reader and creation behavior for issue 35. If the order
is device-local or the capture is inconclusive, record that limitation. A writable ordering
change requires a separate bounded Twydo-to-phone check and approval before upload; this
baseline alone does not establish write compatibility.

## Capture results

### 000-created

User created and synced the agreed synthetic checklist. A read-only CalDAV REPORT against
the approved collection captured only the matching parent and its three children into a
private temporary directory. The normal desktop vdir, sync state and configuration were not
changed. No uploads, discovery, completions or deletions were performed. Credentials were
read through the existing configured credential helper and were not included in fixtures.

The four synthetic resources are in
[`issue35-checklist-order/000-created`](../../../fixtures/phase0/v1/issue35-checklist-order/000-created/).
The parent and all three children have `DisplayOrder: 0`; no sort field is present in their
metadata. Child `CREATED` timestamps are 16:55:04, 16:55:12 and 16:55:21 UTC for Zulu,
Alpha and Mike respectively. This distinguishes creation order but does not yet prove that
the phone sorts by it in every case.

Desktop default sorting would place these undated items as Alpha, Mike, Zulu. The user
confirmed that the phone instead displays Zulu, Alpha, Mike, matching creation order,
and that drag reordering is unavailable. No reorder action or capture was performed.
The reorder path stops here as planned. Proposed next action: retain the same checklist,
add Bravo on the phone and capture its placement; no sort/account changes or cleanup.

### 010-appended

The user added Bravo, confirmed it appeared at the end and synced. A second read-only
REPORT captured the same parent and original children plus Bravo, with no local vdir,
normal sync state/configuration or server changes by the agent.

[`010-appended`](../../../fixtures/phase0/v1/issue35-checklist-order/010-appended/)
contains Bravo with `DisplayOrder: 3` and `CREATED:20261105T170445Z`. The original three
items retain `DisplayOrder: 0` and their original creation timestamps. Their modification
timestamps and property layout changed between captures, but no other logical fields did;
this capture does not establish what phone interaction caused those timestamp changes.

The phone display is Zulu, Alpha, Mike, Bravo. This supports ascending creation time when
ordering values tie and establishes that a phone append can write the zero-based position.
It does not independently prove the precedence between creation time and `DisplayOrder`,
because the two agree in these captures. No manual reorder capability was demonstrated.

### Project 000-created

The approved project extension captured four new synthetic resources using the same
read-only server request, filtered to the project family. No checklist resources were
saved again, no server writes occurred, and normal desktop vdir/state/configuration were
unchanged. Fixtures are in
[`issue35-project-order/000-created`](../../../fixtures/phase0/v1/issue35-project-order/000-created/).
The parent has `TaskType: 2`; it and all three items have `DisplayOrder: 0`. Zulu, Alpha
and Mike have ascending creation timestamps 17:10:41, 17:10:49 and 17:10:56 UTC.
The user subsequently confirmed the project's display after appending as described below.

### Project 010-appended

The user appended Bravo and synced, confirming display Zulu, Alpha, Mike, Bravo. A read-only
capture found exactly one new project child. Bravo has `DisplayOrder: 3` and
`CREATED:20261105T171317Z`; the parent and original three children are byte-unchanged.

[`issue35-project-order/010-appended`](../../../fixtures/phase0/v1/issue35-project-order/010-appended/)
stores only Bravo. Its `step.toml` references `000-created` and explicitly lists the four
unchanged resources; reconstruct this state by overlaying the new file on that baseline.
No checklist files or unchanged project resources were duplicated. No server writes,
normal desktop sync/configuration changes, completions or deletions occurred.

## Approved implementation and local verification

The user approved the implementation design and instructed continued work. The reader
uses `DisplayOrder`, then ascending creation time in both tree and
flat views, independently of the containing view's sort. Ordering-field precedence is an
inference from its documented meaning, rather than a separately captured phone reorder.
Independently displayed project items retain their flat view's date sort. New checklist
and project items receive an append position matching the phone's observed zero-based
position. Equal/missing creation times fall back to UID; second-resolution timestamps
cannot establish entry order on their own. Writer changes need a separately approved
Twydo-to-phone check.

Local fixes are split into display ordering (`6b32a95`), guarded append creation (`8ceede6`)
and the next input row's placement (`c3130b4`). New regressions were observed failing before
the corresponding fix. Initial validation passed all 864 Rust tests, 49 Python tests,
documentation links, clippy with warnings denied and formatting checks.

Independent review found that the Details pane still used date/title sorting. A new rendered
regression reproduced the mismatch for both captured parent types; Details now shares the
core item comparator with the task list. Its 42 rendering tests pass. Final validation passed 865 Rust tests,
49 Python tests, documentation links, clippy with warnings denied and formatting checks. The review found no other Critical or Important issue. One minor remains:
legacy items tied on position and creation time fall back to UID, which changes on moves
and history copies. Such tied historical order is not recoverable from the captured fields;
no normalization of old metadata is enabled by this investigation.

Two candidate resources were generated privately through the candidate CLI in verified
mode: one Echo under each synthetic parent, each with `DisplayOrder: 4`. The ten existing
family resources remained byte-unchanged; automatic sync was disabled by omitting sync
configuration in the isolated candidate environment. No candidates have been uploaded.
The candidate manifest, hashes, local config and journal remain outside git.

Proposed bounded return: fresh-read the two known families, refuse stale dependencies,
then upload only the two new Echo resources with create-only preconditions. Ask the user
to confirm both appear last, rename each Echo to ECHO, and sync. Capture only changed/new
resources, referring to existing evidence for unchanged members. No cleanup, completions,
deletions, other family writes, account changes or normal configuration changes. Await
fresh approval before upload; the earlier read-only approval does not authorize it.

### 020-twydo-appended: approved upload and server read-back

The user explicitly approved the two-item writer check. A fresh read found the exact
expected two parents and eight existing items, with all ten dependency hashes matching
the reviewed private candidate manifest. Only two PUT requests were made: one new Echo
under each approved parent, both guarded by `If-None-Match: *`; both returned HTTP 201.
Fresh server read-back verified both new resources byte-for-byte against the candidate
CLI output and all ten existing resources byte-unchanged. No normal local vdir, sync
state/configuration, accounts or other server resources were changed by these actions.

Each scenario's `020-twydo-appended` delta stores only its new Echo resource at
`DisplayOrder: 4`, referring to its existing five family resources rather than copying them:
[checklist](../../../fixtures/phase0/v1/issue35-checklist-order/020-twydo-appended/),
[project](../../../fixtures/phase0/v1/issue35-project-order/020-twydo-appended/).
The writer revision is recorded in each step; the later Details-pane fix does not change
the writer. Private manifests, receipts, dependency hashes and account paths stay outside git.

Phone return pending: user syncs, checks both Echo items are last, renames each to ECHO
and syncs. Do not repeat creation or upload. Store only changed/new return resources and
reference earlier captures for unchanged members. This check does not authorize cleanup,
completion, deletion, reorder experiments or any additional test tasks.
