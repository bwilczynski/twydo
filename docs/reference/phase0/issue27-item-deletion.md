# Issue 27: representative individual-item deletion session

Status: **four-example scope approved; phone return 040 passed; item-delete verdicts enabled; complete; representative interop, shipping permissions and review fixes verified**.

This is the individual-item deletion increment of [Plan 8](../../superpowers/plans/2026-11-04-plan-8-editing-completeness.md), targeting [issue 27](https://github.com/bwilczynski/twydo/issues/27).

## Scope and reasoning

Deletion removes one selected resource while preserving its parent, siblings,
recurrence, dates, alerts and history. Test that invariant using a few meaningful
representatives; do not build a Cartesian product of recurrence/state combinations.
The user explicitly approved this approach after rejecting the original oversized
session. Its unuploaded 208-resource candidate set was discarded. It must not be
regenerated or uploaded.

Reuse C29's monthly-checklist deletion evidence, existing family captures and
preservation/recovery tests. No repeated RF/RA or issue-41 rounds. Other recognized
recurrence/date/alert combinations get local preservation tests, not new phone cases.
New phone evidence must confirm the expected write set before widening admission.

## Approved collection and preparation

Use only the existing synthetic **Twydo test** collection, privately bound to its
collection ID and the existing shared vdir/state/lock. The user expressly requested
clearing it first. Exact remote bytes/ETags and local vdir/state/config were backed
up privately. Exactly 12 old synthetic issue-41 resources were removed with
conditional remote deletions and two verified local family deletions. The scoped
shared runner reconciled the clear, without forced deletion or metasync; all other
collections and normal configuration remained unchanged.

The phone initially displayed two old parents after sync. A fresh read-only server
check still found zero resources. Reopening the phone list made those entries
disappear; the user confirmed the list is empty and no manual deletion was needed.
This establishes eventual convergence, not the cause of the transient red icon.

No account changes, collection deletion, global write-mode changes, broad sync or
further cleanup. Preserve all private backups and receipts outside git. Baseline
worktree verification passed all 1,030 Rust tests; build output uses a disk-backed
target directory after `/tmp` quota exhaustion during the first build attempt.

## Four examples, two directions, 20 records total

Each example has an `a` phone-baseline family and a `b` Twydo-writer twin. Parent
UID aliases are `twydo-i27-<c|p|m|w>-<a|b>`; child suffixes are `-q`/`-x`. Exact
UIDs, source captures, expected files/hashes and collection binding live in the
private manifest. All records use fresh mapped identities from synthetic captures,
notes `x` and synthetic date-only due 2026-12-10. No alarm editing is involved.

| Parent titles | Shape | Starting items | Records per twin |
|---|---|---|---|
| ca / cb | Plain checklist | open q; completed x | 3 |
| pa / pb | Plain project | independently weekly-repeating open q; completed x | 3 |
| ma / mb | Monthly checklist | only open q | 2 |
| wa / wb | Weekly project | only open q | 2 |

The source fixture seeder remaps standard and JSON identities; core field writers
prepare dates, rules and completion states. Validate all 20 resources using the
actual core export: expected types/statuses/rules, no blocked tasks, correct
RELATED-TO/parentuid links, all internal/series identities mapped, no UID collision.
Seed only this manifest under the shared lock after a fresh empty-collection check;
use a create-only upload or test-only shared sync, then verify the server read-back.

## Two phone phases with separate capture barriers

Agent handles all preparation, captures, comparison, implementation and sync.
Give the user one short batch at a time; never combine the effects at a barrier.

| Barrier | Phone actions | Required evidence |
|---|---|---|
| 000 / 001 | Sync and inspect starting families | Empty preflight and exact 20-record seed/upload; x already completed in ca/pa; b twins untouched |
| 010 | Delete q from ca and pa only | Two selected resources removed; parent, remaining completed x and all other resources unchanged |
| 020 | Manually complete pa | Capture separately: non-recurring parent becomes completed with its already-completed x; no guessed parent/item side effects |
| 030 | Delete x from ca and pa; delete only q from ma and wa | Last-item behavior for both kinds, completed item under open/completed parents, monthly/weekly parents; verify retained parent type/status/rule and exact changed-resource set |
| LOCAL / T20 | None | Add RED→GREEN tests and implement observed invariant; fresh scoped pull; replay same seven operations on cb/pb/mb/wb with per-operation captures; validate and upload |
| 040, phase 2 | Inspect cb/pb/mb/wb, including completed pb; change each parent note x → z | Phone reads and subsequently edits Twydo's empty parents; deleted children stay absent and schedules/history stay preserved |

Expected if deletion is uniform: final capture 030 has 14 records (four empty a
parents plus the unchanged ten b records); final writer/return has eight parent
records. These are hypotheses checked against phone deltas, never evidence by
assumption. Unexpected parent conversion, auto-completion, recurrence change,
resource resurrection, conflict, or out-of-scope write stops the affected batch.

No extra device profiles or phone rounds without explaining the missing evidence
and obtaining approval. Undo/redo/recovery and other recurrence shapes are local
tests; no separate device undo round is required by this revised scope.

## Implementation and closure

Use dedicated item-delete capability evidence, distinct from whole-parent delete
and recurrence execution. If captures establish single-resource removal independent
of scheduling, generalize that operation to recognized flat checklist/project
families and recognized recurrence shapes. Do not require an unrelated completion
verdict to preserve unmodified schedules. Keep malformed resources, duplicate UIDs,
broken links and unsupported nested structures refused with reasons.

Tests in existing gate/store/queue/journal/TUI targets cover exact removal and
preservation (including other known recurrence/end/date/alert shapes, history,
unknown properties/components/JSON identities), last and completed items,
changed/removed/new siblings, changed parent/target, durable dependencies across
undo/redo/recovery, synchronization and strict queued-write preconditions. TUI
checks exercise selected-child `dd`, completed-section targeting, empty-parent
rendering and inverse actions. New behavior tests fail before implementation.

Promote matrix verdicts only after both directions and the phone's subsequent
edits pass. Update compatibility/tasks/TUI documentation and Plan 8's coverage
ledger in the same change. Run focused checks, `mise run test`, `mise run lint`
and `python3 tools/check_links.py`; review privacy and the final write sets.
Make focused commits without agent attribution. Do not post, push, merge or
perform further cleanup without the corresponding authorization. Close #27 only
when supported representative deletions work in verified mode and the generalized
preservation tests pass; do not claim unrelated family/recurrence operations verified.

## 001 seed checkpoint

The fresh collection check found zero members. Exactly 20 approved synthetic
records were then created through one durable journal entry under the existing
shared locks and uploaded through the test-only runner. Server GET read-back
matches every intended resource (allowing only transport line-ending differences);
the journal is confirmed synced. Other collection hashes and normal configuration
are unchanged. Vetted empty/seed snapshots are under
`fixtures/phase0/v1/issue27-items/000-empty` and `001-seeded`; privacy scan passes.
No item deletion or capability promotion has happened yet.

## 010 first two phone deletions

The user deleted q from ca and pa and synced. Scoped reconciliation captured
exactly 18 resources: only `twydo-i27-c-a-q` and `twydo-i27-p-a-q` were removed.
Every surviving resource is byte-identical to 001, including both parents, both
completed x items, all recurring parents and all writer twins. No history or
other resource was created. Other collection hashes are unchanged; privacy scan
passes. This verifies removal of an ordinary checklist item and an independently
weekly-repeating project item without parent/sibling mutation on the phone side.
Last-item behavior and Twydo acceptance remain pending.

## 020 completed-project prerequisite

The phone manually completed pa. The scoped pull still contains 18 resources;
only pa and its already-completed x were rewritten. Pa gains completed status,
100 percent and its completion time. The x item retains its existing completion
time and every semantic value; only modification stamps and serialization
order/folding change. Parent type, dates, identities, metadata and all other
16 resources are unchanged. Other collection hashes are unchanged; privacy scan
passes. This is captured separately from deletion and does not imply that
item deletion itself rewrites a parent or sibling.

## 030 last-item deletion invariant

The four last-item deletions remove exactly ca-x, pa-x, ma-q and wa-q. All
14 surviving resources are byte-identical to 020: empty ca remains an open
checklist, empty pa remains a completed project, empty ma remains an open
monthly checklist, and empty wa remains an open weekly project. There is no
parent conversion, implicit completion/reopen, date/rule change, new history or
resource resurrection. Writer twins and other collection hashes are unchanged;
privacy scan passes. Combined with 010, all six representative phone deletions
are exactly one-resource removals independent of status or scheduling. Candidate
implementation and subsequent phone edits on Twydo output are still required.

## Local candidate checks

The gate now supports dedicated checklist-item/project-item deletion evidence
for recognized same-list flat families, independently of field-edit or recurrence
execution verdicts. Matrices without those new rows retain the earlier C29 path.
Shipping rows have not been promoted. The removal writer still deletes exactly
one resource. Durable parent/sibling and membership checks now cover every item
deletion, including plain families, through undo/redo/recovery. A failing
regression first demonstrated that a plain-family undo could ignore a changed
parent; extending the existing durable dependency condition fixes it.

All 1,040 Rust tests, 49 Python tests, documentation links, clippy and formatting
pass. The mise wrapper could not isolate machine-wide tool resolution inside the
sandbox; its exact project Rust/Python/link and lint commands were run directly.
An offline CLI rehearsal of the seven writer operations passes with six exact
resource removals, only pb completed, eight retained parents and unchanged a
parents. This is local evidence only; no writer upload or phone return is implied.

## 031–038 candidate writer and upload

A fresh scoped reconciliation matched 030 exactly. The seven rehearsed CLI
operations then ran only on cb/pb/mb/wb, using a process-only test configuration
and private baseline-only item-delete rows with the existing shared state/lock.
Per-operation captures contain 13, 12, 12, 11, 10, 9 and eight records. Each of
the six deletions removes exactly one item; only pb changes on completion.
All four a parents, other collection hashes and normal configuration stay
unchanged. Deletion entries retain durable parent/sibling and membership checks.

Scoped upload passed; 038 is byte-identical to 037. Server GET read-back matches
all eight retained parents (only transport line endings differ), and all seven
writer journal entries are confirmed synced. Capture privacy scans pass.
Shipping deletion verdicts remain unchanged until the phone reads these parents
and changes their notes x→z. No extra device actions or cleanup are included.

## 040 phone accepts and edits all four empty parents

The phone inspected cb/mb/wb as open empty parents and pb as completed and empty,
then changed their notes x→z and synced. Exactly those four resources change
notes and modification stamps; sorting/unfolding the logical properties proves
every other value, parameter and metadata key is preserved. Parent types, due
dates, rules, series identities and pb completion time are unchanged. All four
a parents remain byte-identical, there are still exactly eight resources, and
no deleted item returns. Other collections and normal configuration are unchanged.

Both directions and the subsequent phone writes now support the dedicated
`checklist-item.delete` and `project-item.delete` verdicts. The new shipping
permissions generalize the single-resource removal invariant rather than enabling
unrelated scheduling operations. Final TUI/suite/review checks follow.

## Shipping-matrix validation

The actual verified-mode TUI writer now passes selected open/completed/last-item
`dd`, exact-byte undo/redo, empty-parent visibility and queued deletion followed
by undo. Completed checklist items are selected in All/list Done sections, in
accord with existing view behavior; no view implementation change was required.
The new default-matrix gate tests pass all captured shapes, while explicit legacy
matrices retain C29 constraints. A field-edit regression reproduces all four
phone note writes and preserves every other logical property and native folding.

Final validation: 1,043 Rust tests and 49 Python tests pass; clippy, formatting,
documentation links and capture privacy scans pass. Two existing matrix-count
checks now expect the two added rows (115 total). Independent branch review
follows before final closeout. No further device action is needed by this scope.

## Independent review and guard fixes

The independent review found two important guard gaps, reproduced before fixes:
multiple UID/parent properties were lost by the read projection, and a raw-byte
parent search could miss a newly joined item whose UID link was validly folded.
No critical or minor findings were reported.

The read model now keeps those ambiguous resources visible but marks them
read-only; selected-item deletion cannot act on their first projected identity.
Membership validation parses/unfolds resource links before comparing them.
Regressions first failed, then passed for ambiguous resources and folded-link
stale apply, queued delete, undo, redo and recovery before/after removal.
An unreadable resource outside recorded member paths now refuses conservatively,
because its membership cannot be determined safely. No device recheck is needed
for these admission/recovery guards; the writer's one-file deletion is unchanged.

Review set-asides: existing flat Done-view omission of checklist items is unchanged
(All/list Done-section selection is tested); Cartesian phone coverage was
explicitly replaced by the approved four-representative scope. Neither is a new
regression or grounds to expand this device session.

## Final closeout

Both important review findings are fixed with reproducing RED→GREEN regressions.
Post-fix validation passes all 1,045 Rust tests, clippy, formatting and documentation
links; all 49 Python tests and capture privacy checks pass. No unresolved critical,
important or minor review finding remains. The source and evidence are committed
on `issue27-item-delete`; integration and GitHub issue closure remain the user's
choice. The eight synthetic empty parents remain; no additional device action,
cleanup, account change or global write-mode change is part of this closeout.
