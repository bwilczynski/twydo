# Issue 35: checklist and project item order

Status: approved and implemented, 2026-11-05. The separately approved two-item writer
return passed: both items displayed last and retained identity/order after phone edits.

## Intent and evidence

Checklist and project items should retain their sequence and newly added items should append.
The user requested the same behavior for projects on 2026-11-05.
The [bounded captures](../../reference/evidence.md) establish phone
display Zulu, Alpha, Mike, Bravo. The first three have `DisplayOrder: 0` and ascending
creation timestamps; appending Bravo writes `DisplayOrder: 3`. Drag reorder is unavailable.
Twydo currently applies the containing view's sort to checklist children, with alphabetical
ties, and writes zero for new items. This also moves the next input row into the middle.

Project ordering now has its own bounded phone baseline and append evidence: it displays
Zulu, Alpha, Mike, Bravo, with Bravo at `DisplayOrder: 3` and the original three at zero.
The append capture stores only the new child, referencing four byte-unchanged baseline
resources. The user requires evidence to avoid duplicating unchanged files or other families.
The reference documents per-project
sort settings, but no verified shared CalDAV representation for those settings is currently
read by Twydo. The requested behavior is an insertion sequence for items shown under either
parent type; adding a project-specific sort editor is outside this change.

## Approaches

- Creation time alone matches the captured display, but ignores the ordering value the
  phone writes and cannot sequence items created within one second.
- **Recommended:** use `DisplayOrder` then creation time for checklist and project children,
  and assign an append position to newly created items. It uses both observed fields and
  resolves rapid-entry ties without changing creation timestamps. Ordering-value precedence
  is an inference from its documented purpose; the captures do not distinguish precedence.

## Proposed behavior

- In All/list tree views, children under a checklist or project, and in
  Today/Scheduled/Next 3 Days flat views, children nested under a checklist,
  sort by numeric `DisplayOrder` ascending, treating absent/invalid values as zero, then
  creation time ascending (missing last), then UID for deterministic ties. UID fallback
  cannot recover unknown historical entry order. Top-level tasks retain their current sort
  behavior, including project children appearing independently in flat date views. Project
  items retain independent dates, recurrence, eligibility, grouping and notifications.
- Creating a checklist or project item assigns the larger of the existing sibling count and one
  more than their largest nonnegative ordering value. An empty checklist starts at zero.
  Count all siblings, including completed items. This matches the captured append to three
  zero-valued siblings and keeps new items beyond sparse existing positions. Guard numeric
  overflow. Do not rewrite siblings, timestamps or unrelated metadata.
- An Enter chain opens the next blank row at the end of the parent's visible items,
  including after an item whose timestamp is tied or missing. The cursor still follows
  the saved item. Existing `o`/`O` initial row placement remains unchanged.
- Preserve ordering values through edits, undo/redo, family copies, completion/reset and
  sync. No drag UI, J/K manual ordering, project sort preferences or account changes.

## Validation and compatibility boundary

Use the captured families in existing view tests, covering tree and flat layouts and list
sort overrides. Exercise equal/missing timestamps, malformed/absent ordering values,
same-second creation, append positions, completed siblings, stale sibling dependencies,
undo/redo and unchanged sibling bytes. Exercise the Enter chain through the real writer.
Update user documentation and run workspace tests, lint, Python tests and links.

The separately approved project baseline is complete: a non-recurring
project `Project Order 35` in the same `Twydo test` collection, with undated non-recurring
items Zulu, Alpha, Mike added in that order, then Bravo. Both barriers are captured and the
displayed sequence confirmed. No drag attempt or sort-setting changes were performed.
Only these synthetic family resources became fixtures. Do not repeat these phone actions.

The reader is local; the new writer representation needs a bounded phone return before
being described as interoperable. Proposed additional scope: create exactly one synthetic
item `Echo` under each of `Order 35` and `Project Order 35`, upload only those two new
resources after fresh reads, then ask the user to confirm each is last and rename each
to `ECHO` on the phone and sync. Capture the return and verify identities, parent links
and ordering values. No other writes,
deletions, cleanup or configuration changes. Obtain separate user approval for this scope
before any upload. Stop on stale resources, unexpected changes or a display mismatch.
