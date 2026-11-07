# Incremental capability gating

Date: 2026-11-07 · Status: approved (2026-11-07)

## Intent and chosen approach

Make ordinary Twydo actions available without requiring a Cartesian product of
phone experiments, and make compatibility reporting describe actual permissions.
The user chose incremental migration over rebuilding the complete matrix/gate
schema. A global permissive mode was also considered and rejected as the default:
it would discard useful operation-specific protection rather than improve it.

This design amends the [main spec](2026-10-31-terminal-tasks-design.md) §5–6 compatibility policy and §9 check reporting
once approved. Preservation, conflict, recovery and notification contracts remain
binding. Existing phone-session approvals do not authorize additional device work.

## Policy

An operation is admitted on its actual write contract: resources and fields it
changes, identities/relationships it depends on, and invariants it preserves.
Unchanged scheduling fields do not require recurrence execution/edit verdicts.
A broad task shape is not by itself a reason to demand new interoperability proof.

Each migrated capability documents:

- Allowed field/resource changes, including modification stamps.
- Fields/resources that must remain unchanged and structural prerequisites.
- Direct phone evidence and the explicit reasoning for generalizing it.
- Preservation and queue/recovery regression coverage.
- Concrete exclusions and their failure mechanisms.

Directly verified and supported by preservation evidence are distinct evidence
labels. Both may authorize an operation within its contract. Missing observations
for every task combination do not automatically imply refusal. Known failures
remain applicable to operations that exercise the failed behavior; unrelated
byte-preserving operations are evaluated independently.

Keep the current matrix and existing gate paths during migration. Add narrow
capability paths as needed; do not flip broad rows to pass and thereby enable
unrelated operations. Established item deletion and DisplayOrder-only reordering
are precedents, not new work to repeat. Their evidence is recorded in the
[item-deletion session](../../reference/phase0/issue27-item-deletion.md) and
[item-reordering session](../../reference/phase0/issue41-item-reorder.md).

## Proportional verification

Text/order edits need representative format/interoperability evidence and local
proof that identity, links, scheduling and unrelated metadata remain intact.
Removal/relocation also needs evidence for resource disappearance, relationship
changes and empty-parent behavior where relevant. Scheduling changes need targeted
evidence for their actual execution semantics: dates, anchors, ends, alarms,
child resets and history as applicable.

Every additional phone case must name an unresolved failure mechanism and explain
why existing captures or a local test cannot answer it. Recurrence/status/date
combinations are not separate cases unless the operation couples those features.
Reuse existing fixtures first. Broad automated preservation coverage is welcome;
copying it into manual phone rounds is not required. New device scope is bounded,
separately approved, and stops on unexpected effects.

Ambiguous identities, unreadable data needed by an operation, unsafe relationships,
stale dependencies, conflicts and incomplete recovery remain hard safeguards.
Unknown unrelated properties may be preserved without understanding them. This
policy does not authorize destructive actions on resources of ambiguous identity.

## First implementation increment: truthful reporting

Replace `GATE: PASSED` with separate data-health, recurrence-evidence and operation
availability output. Rule-level verdicts remain informative, never a promise that
all operations work. Rename `blocked` to describe parser/model-level read-only
resources. Include completed tasks in health/permission analysis where applicable.

Share permission probes between `tdo check` and `tdo op check <uid>`, invoking the
same gate evaluator used by writes, with configured write mode and the pending
queue overlay. Avoid introducing a parallel permission table. Report title and
notes edits separately, completion for open tasks, reopening for completed tasks,
and deletion. Distinguish inapplicable actions from refused actions.

Do not fabricate generic date/repeat edits, create shapes or move destinations.
State that these require proposed values/destination; label any specific probe
explicitly. `op check` must no longer imply a plain new task proves creation of
an equivalent family, or a title edit proves arbitrary editing is permitted.
The report is a permission snapshot, not a guarantee against later conflicts or
changed dependencies. Summaries group refusals by action and reason; task titles
and paths stay behind --verbose. Empty input says there are no tasks to assess.

Exit codes: 0 means the report was produced; 2 means invocation/configuration/read
failure. Retain this reporting-command convention and document it explicitly;
operation restrictions are report content. A missing explicit --matrix is a
configuration error rather than a silently substituted empty matrix.

No new exact-operation dry-run interface is needed in this first increment.
Document it as a follow-up, including store preconditions beyond gate admission.

## Following increments: unblock useful operations

After reporting, audit title/notes edits first, especially recurring checklists
and repeat-from-completion tasks where current captured-shape checks reject text
changes for scheduling characteristics. Reuse captured field writes and prove
unrelated bytes/metadata remain unchanged before introducing a narrow text path.
A mixed save still needs every changed field's applicable contract.

Then inspect standalone/whole-family deletion and moves separately. Preserve any
real relationship, sync or history constraints; do not assume moving equals
renaming or deleting. Prioritize by actual refusal reports and user needs.
Do not replay completed item-deletion or ordering phone rounds.

The audit records the specific blocker, actual writer behavior, reusable evidence,
minimum missing proof and resulting enablement boundary. If existing evidence is
sufficient, local verification can enable the capability without another phone
session. If insufficient, present only the unresolved bounded experiment.

## Validation and documentation

First increment regressions reproduce healthy-looking data with refused actions,
partial text permissions, completed/history tasks, configured write modes, pending
queue state, malformed resources, missing matrices, privacy defaults and empty
input. Compare reporting with real gate decisions. Preserve writer behavior in
this increment. Update compatibility docs, command help and relevant spec text.

Subsequent gate changes require tests proving their exact write set, preserved
content and relevant stale/queue/undo/recovery behavior, with a small set of
negative neighbors. Reuse existing recovery infrastructure coverage rather than
retesting every schedule combination. Run focused tests and the project's required
checks; update matching as-is documentation with each behavior change.

Implementation details left to the builder: report structs/formatting, helper
placement and test fixture selection. No new dependencies or global write-mode
changes are expected. No production data/server/phone access is part of this work.
