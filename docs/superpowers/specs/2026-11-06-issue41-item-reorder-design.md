# Issue 41: reorder checklist and project items

Status: approved, 2026-11-06, including the Twydo-originated compatibility contract.
Shared `DisplayOrder` storage chosen by the user. Rebased onto main at `9e5f1d7`
before implementation; use its write queue for saves during sync.

## Intent and scope

Move items up/down within their checklist or project and retain that display sequence
after reload, restart and sync. Implement `J`/`K` without introducing sorting controls.
Preserve actual `o`/`O` insertion positions within parents; `a` and subsequent Enter-chain
items append. Top-level ordering, sort preferences, reparenting and indent/outdent are out
of scope. Dates, recurrence, completion, eligibility and counts keep their existing semantics.

## Approaches

- **Chosen: shared `DisplayOrder`.** Reuse the nested-item comparator and the journaled
  resource writer. This can share order with 2Do, but changing existing item positions needs
  compatibility evidence beyond the issue 35 append checks.
- Local ordering would avoid new shared writes, but needs a second ordering mechanism and
  would leave the phone's sequence unchanged. The user chose shared storage instead.

## Proposed controls and sequence

- In the Tasks pane, `J` moves the selected nested item after the next visible sibling;
  `K` moves it before the previous visible sibling. Neighbors must be nested under the same
  parent in the same displayed section. No wrapping. At either boundary, show a short message
  and produce no journal entry, undo step or sync trigger.
- Search may hide siblings. Move relative to the visible neighbor in the complete saved
  sibling sequence; hidden siblings retain their relative order. For example, saved A,B,C,D,
  with B hidden: moving A down past C produces B,C,A,D. Do not exchange unrelated rows or
  move between sections. Details reflects the resulting full sibling sequence.
- Independently displayed project items in flat date views, detached children in another
  section, top-level tasks, the Sidebar and Details panes do not offer these move controls.
  Explain the context restriction in the status message/help. Overlays never pass moves
  through to underlying tasks. Keep selection on the moved UID after reload.
- For `o`/`O` on an item, save immediately after/before its anchor in the complete sibling
  sequence. Carry the anchor through Tab into the edit card and refuse a stale/deleted or
  reparented anchor. `a` and the Enter chain retain append behavior. Standalone creation is
  unchanged.

## Proposed write behavior

Use core operations carrying item/anchor UIDs and placement, with the TUI resolving the
visible neighbor. The store validates that both belong to the same declared parent and
derives the full sequence using the existing comparator: position, creation time, UID.
Include completed/cancelled siblings, so completion does not discard saved position.

For an explicit move or anchored insertion, materialize the resulting sibling sequence as
zero-based integer `DisplayOrder` ranks, writing only resources whose saved numeric rank
must change. This deliberate operation may assign distinct ranks to legacy ties; loading,
ordinary edits and appending never normalize existing metadata. Do not change creation
timestamps or claim to recover historical insertion order. Preserve unrelated resource and
metadata bytes using the existing field editor; apply normal modification timestamps only
to changed resources. Reject unsupported metadata rather than converting its format.

One action is one durable journal entry and one undo step. Reuse atomic writes, recovery,
undo/redo and sync triggering. Hold the parent and all unchanged siblings, and persist
family-membership guards and dependencies through recovery. Refuse the entire action if
any dependency changed or a sibling joined/left after loading. Undo after remote changes
must refuse rather than overwrite them. No partial reorder is exposed to local readers.

## Compatibility boundary

Issue 35 proves creation/append positions and subsequent phone rename preservation, not
reordering existing items or arbitrary position precedence. Phone-side reordering is
unavailable, as already established in that session and confirmed by the user. Do not
repeat drag attempts or require an iPhone-originated reorder baseline. The interoperability
contract is that the phone displays and preserves ordering written by Twydo.

Add dedicated compatibility rows for reordering and anchored insertion; keep them unverified
until evidence exists. Represent the unavailable phone-originated reorder explicitly as
unsupported/not applicable, not as a missing test or a blanket refusal of Twydo's operation.
Use the existing captured `DisplayOrder` representation as the starting baseline and require
the Twydo-to-phone display and phone-edit preservation evidence below to enable this
specific operation. If the current matrix/gate assumes every operation must originate on
the phone, adapt it narrowly for this contract; do not mark an unperformed phone reorder
as passing or relax other operations' compatibility requirements.
Verified mode refuses these new writes while unverified. Existing append behavior stays
available under its current gates. Tests use synthetic resources and explicit test matrices
or the existing experimental mode; never change the user's configuration or write mode.

Prepare an exact bounded device checklist only after the local candidate is reviewable,
and obtain fresh approval before accessing the NAS/iPhone or uploading candidates:

1. Twydo reorders synthetic siblings into a sequence that differs from their unchanged
   creation order, then uploads only the agreed changed resources with stale-data guards.
2. The user syncs the phone and confirms that it displays Twydo's sequence. The user does
   not attempt to reorder anything on the phone.
3. The user makes an agreed unrelated edit, such as renaming one reordered item, and syncs.
   Capture the return and verify the phone preserves item identity, parent links, saved
   ordering and the displayed sequence; Twydo must read the returned sequence correctly.
4. Check anchored insertion separately: Twydo inserts before/after an existing sibling;
   the phone displays that placement and preserves it through the agreed edit/sync return.

Cover checklist and project families separately and store only new/changed evidence
resources. Keep the direct phone sample explicit. Reordering is a field-specific operation: its
verified DisplayOrder representation applies to structurally valid checklist/project
families while recurrence, dates, alarms and statuses remain byte-preserved. Recurrence
execution and new-task creation keep their own compatibility checks. This checklist describes the evidence required, not permission
to perform device/server actions.

## Validation and documentation

Test both parent types, visible/hidden siblings, first/last items, overlays, detached rows,
selection restoration, insertion through the row and edit card, tied/missing/malformed ranks
and timestamps, completed siblings, unchanged task semantics, stale family membership,
recovery, undo/redo and unrelated metadata preservation. Verify gating independently of
the local ordering algorithm. Run relevant suites, then workspace tests, formatting, clippy,
Python helper tests and documentation links. Update TUI, tasks, architecture, data-model
and compatibility pages with the enabled/refused behavior.

Implementation remains Rust in the existing workspace, with no new dependency. Exact
operation/type names and test decomposition belong in the implementation plan. Production
data and device access are not authorized by this design.

## User correction: recurring families (2026-11-07)

The user rejected the simple-family-only enablement: their checklists are recurring and
this restriction defeats the requested feature. The initial implementation incorrectly
required recurrence/date/status-specific rows for a DisplayOrder-only operation.

Correct the gate at the operation boundary. Validate editable JSON, known checklist/project
structure, identity/parent links, same collection and non-nested items; then require the
verified checklist/project reorder or insert row by parent kind. Pure reordering does not
parse, edit or execute recurrence and must not depend on recurrence/date/status verdicts.
Anchored creation must additionally pass the existing complete `Create` gate, because it
creates a resource and may inherit recurrence-family dates. These checks remain distinct.

The phone sample establishes rank display/preservation with Manually selected. Tests over
existing recurring checklist/project captures must prove that reordering changes only
DisplayOrder and modification stamps, preserves dates/alarms/status/recurrence metadata,
supports queue/undo and retains the sequence through already-supported family completion.
Document the extension as field-preservation reasoning plus local captured-family tests,
not a new phone round or newly verified recurrence execution. No further NAS/iPhone access
is authorized by this correction.
