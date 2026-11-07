# Plan 8: close the task-editing compatibility gaps

Date: 2026-11-04. Status: batch 1, RF/RA and bounded C29 complete (B-only C29 through 031);
individual-item deletion now adds two operation-level permissions alongside narrow until execution. Later batches/full TUI surface remain pending.
See the session's regression map and local closeout for exact enabled/refused shapes.

## Resume checkpoint — start here after the required project/spec reading

- Latest increment: **issue 27 individual-item deletion**, branch `issue27-item-delete`
  based on `9f9f40d`. The [four-example session](../../reference/phase0/issue27-item-deletion.md)
  completed phone baseline 000–030, Twydo writer/upload 031–038 and phone return 040.
  `checklist-item.delete` / `project-item.delete` generalize the single-resource removal
  invariant to recognized flat families, including completed/last items, without enabling
  unrelated recurrence execution. **Do not repeat the device rounds.** Exact preservation,
  stale-family, inverse/recovery/queue and actual TUI key tests cover the implementation.
  The formerly scoped C29 deletion limits below are historical; other operation limits remain.
  The existing Twydo test list was cleared once with explicit user consent and fresh backups;
  the eight new empty synthetic parents remain, with no further cleanup authorized.
- Earlier C29 increment branch: `feat/29-repeating-checklist`, based on `a047f71` on main.
  Earlier batch 1/RFRA implementation is already in the base; do not replay it.
  C29 phone evidence is complete through B-only capture 031; local promotion closes
  three scoped operation rows. The full Plan 8 surface remains unfinished.
- Batch 1 phone work is complete through 153. **Do not repeat those phone rounds.** Five
  logical synthetic cases yielded eight newly passing operation rows, narrow date-only
  daily/plain-monthly until execution, and its end-date summary edge. Exact scopes and remaining
  refusals are in [compatibility.md](../../compatibility.md#what-is-verified-today).
- C29 final validation passed 850 Rust tests, 49 Python tests, links and project lint.
  Shipping-matrix/TUI validation and the deferred minor are in the C29 closeout.
  These checks cover batch 1, RFRA and the bounded C29 slice, not the full remaining TUI surface.
- Assertions live in existing behavior suites (`edit_fields`, `edit_complete`, `store`,
  `store_family`, `gate`, `ical_roundtrip`; TUI in `tdo/tests/write.rs`). Shared fixture helpers
  are in `common/recurrence.rs`. **Do not create session/batch-named test targets.** See the
  [regression map](../../reference/phase0/plan8-tui-completeness-session.md#regression-test-map).
- Matrix accounting after issue 27: 115 operation rows, with 81 iPhone-baseline passes,
  28 unverified and 6 unsupported; Twydo-to-iPhone has 81 passing and 34 unverified.
  This includes the four intervening ordering rows; unsupported phone reorder controls
  do not imply unsupported preservation of Twydo-written ranks.
- Historical matrix accounting after C29: 109 operation rows. iPhone baseline has 79 passing,
  28 unverified and 2 unsupported; Twydo-to-iPhone has 75 passing and 34 unverified.
  These are row counts, not remaining-feature counts: unused attribute placeholders,
  overlapping reopen cases, absent rows and guarded combinations need separate review.
- Private production session manifests/backups are outside git. Existing synthetic test lists
  remain; no cleanup, account/config change or additional production operation is authorized
  by this checkpoint. The five conflicts at 130 were resolved using the user's phone versions.
  Acceptance comes from the clean 141–144/151–153 retry, not that conflicted attempt.

### RF/RA completed; next bounded increment

The user approved a minimum RF/RA evidence set: five twin pairs, ten tasks, with
batched phone actions. Baselines 202–300 and returns 310–380 are complete. **Do not
repeat them.** Scoped `from-completion.edit` and `from-completion.repeat` now pass;
see [the RF/RA checklist](../../reference/phase0/plan8-rf-ra-checklist.md) for exact
capabilities, fixture/test map, writer-equivalence argument and refusal neighbors.
Tests use the shipping matrix directly, including CLI/card interval changes,
date-key undo/redo and atomic grouped-save refusals retaining typed text.

The fields retry's live advance reached the server with a later note save under
the original UID/RUID; its unchanged December 10 history and December 4 live output
match the core writer exactly. The user's earlier reopened occurrence remains
preserved and excluded from clean completion evidence. Review also added a
refusal for setting completion recurrence on plain tasks with a foreign series
link, while supporting opaque foreign ids on existing live recurring tasks.

Two non-session lists were intentionally emptied on the server, as confirmed by
the user. They were not force-deleted locally. The session used private scoped sync
through the same runner/locks, selecting only the two capture collections with
forced deletion prohibited. Normal config, account and write mode are unchanged.
Private manifests/backups and all extra synthetic copies remain outside cleanup.

The next approved bounded increment prioritizes issue #29: see the
[bounded repeating-checklist checklist](../../reference/phase0/plan8-checklist-29-checklist.md).
It includes individual checklist-item deletion, an empty-list preflight and two
bounded phone rounds. The user approved that exact scope and confirmed backup/
empty-list readiness. B-only baseline captures 000–012 and candidate writer/upload
020–027 and phone returns 028–031 are complete. Three scoped rows are now enabled
locally: recurring-checklist create/edit and recurring-checklist-item delete.
Verification passes and the bounded C29 scope is complete; do not repeat any C29 phone actions.
No leftovers cleanup or broader device operations are authorized.

Next: prepare a minimum bounded checklist for the remaining Task 2 work (repeat
end changes or project/item repeat cases), consulting the full session coverage
ledger first. RF/RA does not authorize additional device operations, move/delete,
family work, arbitrary rule shapes or cleanup. Keep later increments separate and
obtain fresh approval for their exact production scope. The written checklist
below applies to that next increment, not to replaying completed RF/RA rounds.

Before involving the phone, reuse prior fixtures, inspect the actual gate/writer paths, and
prepare one concise checklist showing:

1. Exact remaining actions and synthetic UID targets, with reused evidence identified.
2. The specific field/anchor transitions to prove and expected resource changes.
3. Named capture barriers and a fixed planned number of phone return rounds; explain any
   necessary expansion before requesting it.
4. Exact capabilities that will be enabled, their negative neighbors, and the stop condition:
   implementation + both device directions + ordinary behavior tests + verified-mode TUI
   checks + matching docs, or a precise unsupported finding.

Get fresh approval for that exact production scope before seeding/writing/syncing/phone work.
Pull and validate the latest phone state before each writer round; the user's report of phone
changes is not approval to repeat them on stale desktop copies. Preserve remote edits and both
conflict versions. Keep commits focused, and push updates to the existing PR.

The initial audit and unchecked multi-batch checklist below predate this closeout; they do not
mean batch 1 is unfinished. The checkpoint, current compatibility page/matrix/gate and session
local closeout identify the present state. This is a handoff within Plan 8, not a new plan or a
claim that all existing TUI actions are complete.

## Goal and boundaries

Make supported tasks usable through their whole lifecycle: create, edit, change/remove repeat,
complete, reopen, move and delete. Preserve 2Do interoperability, history, undo and recovery.
Do not simply turn unverified matrix cells into passes or relax verified mode.

Plan 7's implementation and sessions A–C are complete per `AGENTS.md` and its capture results;
its original unchecked step boxes are not evidence that those tasks remain unimplemented.

Sources: [main spec](../specs/2026-10-31-terminal-tasks-design.md),
[add-and-reshape design](../specs/2026-11-03-tui-add-and-structure-design.md),
[current compatibility](../../compatibility.md), [session C](../../reference/phase0/plan7-capture.md),
[`gate.rs`](../../../crates/twydo-core/src/gate.rs),
[`edit/complete.rs`](../../../crates/twydo-core/src/edit/complete.rs),
[`store/family.rs`](../../../crates/twydo-core/src/store/family.rs), and
[`compat/matrix.toml`](../../../compat/matrix.toml).

## Initial audit: missing task operations

“Implemented, gated” means code exists but the complete operation still needs captured evidence;
it does not mean the code is known to produce the right result.

| Gap | Actual state | Required next step |
|---|---|---|
| Complete a repeat with an end date (`until`) | Captured continuation and history, inclusive boundary, and final completion for date-only daily/plain-monthly rules; scoped by the gate | Other until shapes remain unverified |
| Set/remove/reshape daily and yearly repeats | Implemented, `repeat` rows lack Twydo round trips; interval-only edits already work | Capture setting, reshaping and removing, in both directions; keep those actions distinct from interval edits |
| Edit an existing from-completion task | Implemented, `from-completion.edit` has no baseline | Capture title/notes, due/start/time and interval edits; complete afterwards to check the anchor |
| Set/remove/reshape from-completion repeats | Implemented, `repeat` row incomplete | Capture switching from due-date to completion-date recurrence and back, changing class/end, and removing recurrence |
| Set/change/remove a project's repeat | Implemented, `project` and `recurring-project` `repeat` rows lack phone baseline/read-back | Capture non-recurring → recurring → changed → non-recurring with mixed-state items |
| Create a repeating item under a non-recurring project | Implemented, `project-recurring-child.create` lacks Twydo → iPhone verification | Create it in Twydo, inspect/edit/complete it on the phone; capture all family resources |
| Set/change/remove an existing project item's repeat | Implemented, `project-recurring-child.repeat` lacks evidence | Capture separately from creating a new repeating item |
| Complete a recurring project with an end or from completion | Explicitly blocked in every mode; partial store implementation exists | Capture continuing and ending families, history child identities, reset states and dates before implementing/enabling |
| Repeating checklists | All writes blocked by family-shape policy, including creation and conversion to one | First establish whether the phone supports them and capture reset/history/inherited-date semantics; implement only if supported |
| Independently repeating child inside a repeating project | All writes blocked, including creation or converting a family into this shape | Establish phone support, then capture independent completion and parent completion without double advancement |
| Empty project → checklist | Implemented; Twydo output was displayed correctly, but phone baseline/read-back missing | Capture the phone's conversion and subsequent edit of Twydo's conversion |
| Move monthly/yearly repeats | Implemented; top-level `move` rows lack Twydo → iPhone verification | Compare UID/RUID, history references, dates/alerts and destination; phone edits afterwards |
| Move/delete daily, weekly and from-completion tasks | Implemented, operation rows incomplete | Capture each class, retain history, verify phone round trips and undo |
| Reopen a repeat's history copy | Implemented, but additional guard needs a passing monthly or yearly `reopen` row; both lack Twydo → iPhone verification | Capture history reopening separately from restoring/rewinding the live series; verify effects on the original |
| Reopen the last occurrence of an ended series | Explicitly blocked in core and gate; undo works | Capture the phone's actual behaviour, including repeat count/end and subsequent completion, before implementing |
| Move/delete one item on its own | Explicit family-policy blocks; no dedicated verified path | Capture removal from old parent, list/UID/link changes, history and undo; do not infer from whole-family moves |
| Reopen an item of a completed/cancelled parent | Explicit family-policy block | Capture whether parent state changes; distinguish completed from cancelled parents |
| Add an item to a completed/cancelled parent | Explicit family-policy block | Establish whether the phone offers it and what parent state becomes |
| Remove a repeat's due date without removing its repeat | Explicit policy block | Capture whether the phone permits this or fills a date; never produce an uncompletable series |
| Checklist date removal/time/start while items carry inherited date copies | Only replacing the due date with another date-only value is supported | Capture each other change; extend family date cleanup without dropping unrelated item data |
| Nested parents (project/checklist inside a parent) | All writes blocked | Establish phone support first; a separate family-model change if needed, not just a matrix row |
| Custom alert creation/edit/removal | No `Change`/CLI/card editor; existing alerts are preserved and displayed | Capture absolute, relative, multiple and start-relative alarms, add core changes, then CLI and card fields |

### Imported shapes and edge cases

- Recurring completion supports date-only and floating due times, not UTC or `TZID` due times.
  It also refuses absolute alerts, a distinct `DTSTART`, nonzero `StartDayDelay`, unusual JSON
  start dates, missing JSON metadata and inconsistent repeat metadata. Capture valid phone
  variants before adding support; malformed metadata must not be guessed at or normalized.
- Fixed `BYMONTHDAY`/`BYMONTH` rules, other imported RRULE shapes and unknown RRULE parts may be
  preserved/read but cannot necessarily be typed or completed. `step_of` accepts a narrower
  vocabulary than the reader. Support needs shape-specific fixtures, not just a class verdict.
- A missing fifth weekday is **not device-verified**. The implementation currently skips months
  without one, following RFC 5545; the spec explicitly requires observing 2Do rather than
  assuming library/RFC behaviour. Capture a fifth weekday across a missing month.
- Recurrence overrides (`RECURRENCE-ID` or several VTODOs), ambiguous/broken parent links,
  duplicate identities and unknown task types are safety refusals, not ordinary unfinished
  editors. Keep malformed resources protected. Tokens/no-JSON metadata has narrower write
  support (in particular type/repeat changes need JSON).
- `monthly-31st`, `yearly-feb-29`, `from-completion-anchor` and `end-after-n` edge entries were
  stale despite sessions B/C and regression tests covering those behaviours. Task 1 reconciled
  those records from existing evidence. Batch 1 subsequently captured `end-on-date` continuation,
  boundary and final completion for the narrow daily/plain-monthly scope; other until shapes remain gated.

### Do not mistake matrix cells for effective capabilities

The gate combines own-case, family, rule, attribute and shape checks. For example, the
`recurring-project.move` row passes but a monthly/yearly parent's own `move` row still blocks
the operation. Reopening a history copy is also subject to a separate guard. Conversely,
`date-only`, `timed`, `start-date` and `alarms` have unused/unverified operation cells: the gate
only selects date/start attribute rows for creation and relevant edits, and it has no custom
alert editor. Those cells are **not** proof that dated one-time tasks cannot be moved/deleted.

Whole-family operations check children's lookup/shape and recurrence preservation, but do not
individually require every child's operation row. In particular the unverified
`project-recurring-child.move/delete` rows describe whole-family captures, not enabled
standalone item operations. Test actual operations through `gate::check` and `Store::apply`
before changing claims or deciding which matrix evidence is necessary.

### Known 2Do limits: keep refused

- Last day of month, Nth workday (including last weekday), Nth weekend day: 2Do 3.91 misreads
  the tested encodings and cannot create them. No promise to enable them while keeping iPhone
  interoperability. Distinguish “last weekday” from supported “last Friday”.
- A project with items → checklist or task, and the current `parent-with-items-to-task` case:
  marked unsupported. Keep refused; do not silently discard or detach items. The shared case
  was established with projects; any future checklist-specific claim needs its own capture.
- Nth-weekday interval edits overwrite 2Do's ordinal; weekday-set repeats with an interval
  other than one are also refused. Do not enable these by reusing plain interval evidence.
- Children of a plain task and independent dates/repeats on checklist items are not the model:
  use a typed parent and project items respectively, not a bypass of the gate.

## Adjacent editing features still unbuilt

These are separate implementations, not unfinished matrix verification:

1. Indent/outdent (`>>`/`<<`), reparenting and moving into a parent through `m`.
2. Yank/paste duplication (`yy`, `p`/`P`), including whole families and fresh identities.
3. List creation/rename/recolour/archive/unarchive and group management; collection propagation
   needs separate verification. The later design proposes sidebar deletion, conflicting with
   the main spec's v1 ban on deleting collections: obtain a user decision before implementing it.
4. Manual sorting/order persistence and `J`/`K`, including actual saved `o`/`O` placement.
5. Project View in date/smart views (navigation, not a new write encoding).
6. Natural-language extraction on the new-title row with preview (existing date/repeat parsers
   can be reused; do not silently reinterpret ordinary title text).

Notifier/background delivery is also unbuilt, but is not part of task-editing completeness.
Tags, priority, stars, locations, attachments, audio and duration remain outside v1 scope.

## Execution plan

### Task 1 — Audit and stage the work (local only)

- [x] Read Plan 7 results, current docs, matrix, gate, completion and family store code.
- [x] Separate effective gaps, unused matrix cells, imported-shape limits and 2Do failures.
- [x] Record this backlog and an approval boundary before any device/server access.
- [x] Reconcile the four stale edge entries using existing session B/C evidence and their
  fixture-comparison tests; do not change `mod:until.execute` or unrelated operation verdicts.
- [x] Add matrix-backed regression assertions for each reconciled edge record.

Local validation: `mise run lint`, `mise run test` and `git diff --check` passed. No operation
rows or rule verdicts were relaxed; the reconciled edge records are not used by the write gate.
No device, server or production task data was accessed for this audit.

### Task 2 — First device batch: everyday repeat editing and completion

**Expanded session plan:** [finish the existing TUI write surface](../../reference/phase0/plan8-tui-completeness-session.md).
The user requested enough evidence to use all existing TUI features, not only the seven
repeat-editing rounds below. The session plan supersedes that narrow round list and combines
Tasks 2–4's device work into resumable baseline → implementation → final-device-check blocks.
Repeating checklists, lifecycle operations and actual verified-mode TUI acceptance are mandatory;
unbuilt editors/structural features remain Task 5, not prerequisites for this session.

**Approval required before setup, backup, seed, sync or phone access.** Use an isolated synthetic
test account if available. Previous sessions used throwaway production lists: that does not
authorize doing so again. If an isolated phone setup is unavailable, agree the exact alternative,
backup, collections, sync pair, cleanup and reconciliation with the user first. Never globally
switch production writes to test mode. Keep credentials and raw production exports outside git.

- [ ] Confirm target environment with the user and write its session record under
  `docs/reference/phase0/` before running it. Record versions and explicit approval.
- [ ] Seed synthetic twins with `tools/seed_fixture.py` and existing fixtures; no manual typing
  of whole task sets on the phone. Identify resources by UID, not title alone.
- [ ] Run the following small rounds; stop on a mismatch rather than proceeding to enablement:

| Round | Phone baseline | Twydo twin and check |
|---|---|---|
| U | Monthly until: one advance before end, one exactly onto end, then finish | Compare live and history, especially end metadata; inspect in phone, edit/complete again |
| D | Plain → daily → changed class → no repeat | Same transformations, then phone edit/completion |
| Y | Plain → yearly → changed class → no repeat | Same, checking February 29 separately when changing the due anchor |
| F | Edit an existing from-completion task; switch due/completion anchor and remove repeat | Same; subsequent completion proves anchor, not just displayed label |
| E | Empty project → checklist | Compare conversion, then phone edit of Twydo-written checklist |
| P | Non-recurring project → repeating → changed repeat → no repeat, with mixed-state items | Compare every family resource; check child state/date retention |
| I | Create a repeating item under a non-recurring project; set/change/remove an existing item's repeat | Verify independent completion, history links and parent unchanged |

- [ ] Convert each capture into failing fixture-comparison tests in `edit_complete.rs`,
  `edit_fields.rs`, `edit_create.rs` and/or `store_family.rs`; fix mismatches in the corresponding
  core module. Include `until` boundary/end metadata and unchanged unrelated bytes.
- [ ] Add effective-capability tests in `tests/gate.rs`, CLI tests and TUI refusal/save tests as
  appropriate. Mixed allowed/refused card saves remain all-or-nothing with text retained.
- [ ] Enable only demonstrated operation columns/rule verdicts. A single set-repeat capture
  must not stand in for removal, switching anchors or all families without explicit justification.
- [ ] Update `docs/tasks.md`, `docs/compatibility.md` and session results in the same change.

### Task 3 — Lifecycle round trips

- [ ] Capture/verify monthly/yearly moves, daily/weekly/from-completion move/delete, and history
  reopen. Check which history-copy distinctions survive in the stored metadata; do not claim
  class-specific reopen coverage if the copy no longer carries the original rule.
- [ ] Capture ended-series reopen before implementing it. Separate this from undo and from
  reopening a completed occurrence copy; test repeating again after reopen.
- [x] Verify standalone item deletion through the issue-27 representatives and preservation tests;
  dedicated item-delete rows preserve parents/siblings/schedules and empty-parent types/states.
- [ ] Capture standalone item move/reopen and adding to completed parents. Add dedicated
  policy/operation cases where whole-family evidence cannot establish standalone semantics.
- [ ] Test grouped saves, stale family snapshots, undo/redo, interruption/recovery and conflict
  handling for every new multi-resource path. Preserve remote edits on undo.

### Task 4 — Family recurrence and remaining date edges

- [ ] Capture recurring projects from completion and with count/date ends: continuing, boundary
  and final occurrences, mixed-state/cancelled children, history identities and reset dates.
- [ ] Determine phone support for repeating checklists, repeating children of repeating projects
  and nested parents. Record unsupported outcomes instead of guessing an implementation.
- [ ] Implement supported cases with dedicated family and gate tests, then verify Twydo writes
  on the phone. Changing `family_case` alone is not sufficient.
- [ ] Capture missing fifth weekday, recurring due removal, checklist inherited-date cleanup,
  and supported imported date/time/alert shapes. Keep unresolved shapes refused individually.

### Task 5 — New editors and structural features

- [ ] Write a focused design/implementation plan for alert editing, then capture encodings,
  implement core/CLI/card support and verify round trips.
- [ ] Plan and implement indent/outdent/reparent/move, then yank/paste with new family identities.
- [ ] Plan and verify list/group management; resolve collection deletion scope with the user.
- [ ] Plan manual order, Project View and natural-language title parsing as separate increments.

## Acceptance for each increment

- Every enabled operation has baseline fixtures and Twydo/iPhone round-trip evidence with
  justified, narrowly scoped matrix verdicts; unsupported and unverified neighbours remain blocked.
- Test create → edit → change repeat → complete → inspect history → undo/redo → move/delete
  for each newly supported shape, not just the happy-path serializer in isolation.
- No data loss, accidental normalization, duplicate history or partially visible family writes.
- Run `mise run lint`, `mise run test` (includes Python tools and documentation links).
- Update the matching as-is docs and this checklist; do not claim a feature ships before its
  phone check passes. No production migration or device/server action without explicit consent.
