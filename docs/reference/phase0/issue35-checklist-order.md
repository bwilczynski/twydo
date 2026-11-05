# Issue 35: checklist ordering investigation

Status: bounded baseline capture approved by the user on 2026-11-05; creation captured.
Reorder and append captures pending. User requested focused commits.

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
the phone sorts by it. Displayed order and sort setting await the user's report.

Desktop default sorting would place these undated items as Alpha, Mike, Zulu. The next
barrier is the user's report of the initial display followed by dragging Mike ahead of
Zulu, if the phone supports it, and syncing. Do not append Bravo until that state is captured.
