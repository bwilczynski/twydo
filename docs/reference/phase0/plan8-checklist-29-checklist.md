# Issue 29: bounded repeating-checklist session

Status: **bounded scope approved, including individual item deletion; preflight started**. This is the next
increment of [Plan 8](../../superpowers/plans/2026-11-04-plan-8-editing-completeness.md),
prioritizing [issue 29](https://github.com/bwilczynski/twydo/issues/29).

## Outcome and minimum scope

Enable direct creation of a repeating checklist and conversion of an existing
repeating task to a checklist, then adding/ticking/deleting individual items and editing title/notes.
Use only top-level, open, JSON-backed, date-only monthly interval-1 rules, due
anchored, no end/start/alert/delay, due on day 1–28. This is a usable narrow slice,
not complete recurrence vocabulary. Other family operations stay refused.

Reuse batch 1 FC for setting/removing/changing checklist repeats and mixed/all-done
monthly history/reset (010–040, 070–120, 141–153 in `plan8-tui-a`). Reuse Plan 7
plain creation, items and task-to-checklist conversion. Do not repeat those rounds.
New evidence must establish direct recurring creation and conversion preserving
recurrence/series identity, and subsequent item use. Existing tests/evidence do
not establish ordinary recurring-checklist editing or direct creation.

Writer audit: `edit/create.rs::new_task` initializes checklist TaskType/JSON uid,
then uses existing due/repeat setters. `edit/mod.rs::set_kind` updates type/JSON
uid while retaining recurrence. The gate currently requires missing
`recurring-checklist.create` / `.edit` rows, including item creation and conversion.
Any new passing row needs guards distinguishing parent/child operations and exact
captured fields; a passing edit row must not admit uncaptured date/type edits.
`gate.rs::family_op` currently blocks individual item deletion unconditionally.
Replace that refusal only for the captured open recurring-checklist context, with
an explicit dedicated item-delete verdict; do not enable project or whole-family
deletion via the parent row. Audit the store's deletion journal/resource set too.

## Clean-list preflight and approval boundary

Proposed target: only the existing synthetic capture B collection, privately bound
by collection ID and sync pair. Capture A and every other list remain untouched.
Before seed: approved backup, fresh scoped pull through the shared lock/recovery
identity, no pending writes/conflicts, and verify **zero resources**, including
completed/hidden tasks, locally and remotely. Ask the user to confirm the phone
also shows an empty list with completed tasks visible. No inferred emptiness from
an ordinary open-task view. Stop if any leftovers exist; inventory them privately
and obtain specific cleanup approval. This proposal does not authorize leftover cleanup, whole-family deletion
or creating a replacement collection, switching accounts, changing normal write
mode/configuration, or broad sync/discovery.

Bind existing account/device/server versions and private paths without exposing
personal data. Confirm restorable phone backup. Record unrelated collection hashes
privately; validate them at every barrier. Use the same shared state/lock for a
process-only candidate configuration with automatic sync disabled. Never use a
second independent journal/lock over the shared vdir.

## Targets

Four parent branches are sufficient: phone direct `a`, phone conversion `b`,
Twydo direct `c`, Twydo conversion `d`. Each gets items `x` and `q`.
Delete only `q` while the parent and both items are open; retain `x` (renamed `z`)
for the completion/reset checks. Earlier FC already covers mixed multi-item resets.
Bind all four disposable-item UIDs privately before any deletion; approval covers
only these new synthetic items, never leftovers or history. All parents initially due **2026-12-10**, monthly from
due, never ending, date only. Dates remain future but need no typing where the
phone picker can select them. Reconfirm dates if the session is postponed.

Seed only ordinary monthly task prerequisites `b` and `d` using captured fixtures
and fresh collision-checked identities, through `tools/seed_fixture.py`. Twydo
creates `c` directly later; the user creates `a` directly on the phone because
seeding that parent cannot establish phone creation behavior. Proposed fixed
prerequisite UIDs: `twydo-p8-c29-b` and `twydo-p8-c29-d`; record actual seed-generated
UIDs instead if using the seeder unchanged. Allocate/bind `a`, `c` and child UIDs
in the private manifest when created. All operations validate exact UID,
collection, starting bytes and expected changed-resource set before writing.

## Two phone rounds, four baseline and three return capture barriers

These are action budgets, not permission to skip intermediate captures. I perform
all seed/write/sync/capture/comparison work. Phone input uses only single letters; no notes requiring sentences. Give one short instruction batch at a time.

| Barrier | User phone actions | Agent work and expected result |
|---|---|---|
| C29-00 | Confirm empty list, including completed tasks | Fresh scoped pull, zero-resource check, backup, bind IDs; seed/upload only `b` and `d` |
| C29-10, round 1 | Create `a` directly as monthly checklist due Dec 10; convert `b` to checklist | Pull/capture before adding items; two checklist parents retain correct rule/due and distinct identities; `d` unchanged |
| C29-11, round 1 | In each of `a`,`b`, add items `x` and `q`; capitalize the parent title (`a`→`A`, `b`→`B`), set note `x`, rename item `x`→`z` | Pull/capture; four new child resources, correct standard/JSON parent links; only intended parent fields change |
| C29-11D, round 1 | Delete only item `q` from each of `A`,`B` | Pull/capture before ticking anything; exactly two child resources removed, both parents and remaining children byte-unchanged; no history created |
| C29-12, round 1 | Tick each `z`, then tick each parent manually | Pull/capture; parent stays open after item tick; explicit parent tick advances Dec 10 to Jan 10 with correctly linked history and reopened live item |
| C29-20, agent only | None | Replay comparisons/tests; fresh pull; create `c` directly and convert `d`, add `x` and `q`, capitalize titles (`C`,`D`), set note `x`, rename item `x`→`z`, delete only `q`, tick remaining item then parent. Capture each operation locally, verify reset/history and upload. Candidate writes require this exact session approval, normal mode stays verified |
| C29-21, round 2 | Inspect `c`,`d` as open monthly checklists due Jan 10 with open `z` and no `q`; set note `z` on both | Pull/capture; phone acts on Twydo output, preserving type/rule/identities/history and accepting creation/conversion/deletion |
| C29-22, round 2 | Tick each live `z`, then manually tick each parent | Pull/capture; second family reset to Feb 10, correct history, deleted `q` never returns or enters new history, no duplicate or orphan resources |
| C29-23, round 2 | Rename each live `z` to `x` | Pull/capture; phone edits reset Twydo-created child identities/links; parent recurrence and existing history untouched |

Each branch retains its distinct single-letter title, capitalized by the title edit.
Unexpected UI restriction or metadata transition stops the round; capture the
unchanged state/message instead of improvising extra operations. No additional
phone round without explaining the missing evidence and obtaining approval.

## Local completion contract

Before candidate upload, compare direct-create/conversion/parent edits/item
creation, individual deletion and completion with phone baseline under explicit identity/timestamp
mappings. Preserve every other property/JSON key. Do not assume normalization.
Verify individual item completion states before family completion locally; where
phone sync combines actions, existing FC item barriers supply prior evidence and
new evidence must not claim independent timestamps it did not capture.

Use existing behavior targets (`gate`, `edit_fields`, `store`, `store_family`,
`ical_roundtrip`; helpers in `common/recurrence.rs`), no session-named test target.
Failing tests precede implementation. Cover exact writes, whole-family identities,
manual final tick, undo/redo, atomic grouped saves, stale snapshots and recovery.
For each item deletion assert exactly one resource removed, parent/sibling/history
bytes unchanged, no orphan links, and the deleted item absent from later resets
and history. Verify delete undo/redo restores the exact item identity/links; undo
after a simulated remote edit must preserve that edit through conflict handling.
Inspect phone baseline for parent changes/count metadata rather than assuming a
single-resource delete: if the phone rewrites related data, stop and revise the
expected resource set before candidate deletion. Capture the delete barrier
separately from completion so the effects remain attributable.
Negative neighbors: weekly/yearly and interval-2 creation/conversion, end/from-done,
timed/start/alert/delay shapes, date edits, nested parents, project conversions,
child recurrence, item move, project-item deletion, deletion under completed/cancelled
parents, deleting the last/only item, deleting completed/cancelled items, whole-family
deletion and completed-parent item creation. Do not turn
these into passes via a broad family row. Reuse existing repeat/completion support
only within its established guards.

After both directions pass, add narrowly scoped matrix rows/provenance and gate
constraints, real verified-mode TUI key/card tests for creation, `cc`, item add,
item `dd` with exact target selection, title/notes, completion and undo/redo; update as-is compatibility/tasks/TUI docs.
Run `mise run test`, `mise run lint`, and independent review before claiming issue
29 resolved for this scope. If usable creation/conversion still requires a blocked
item action, keep the issue open and state the gap. No claim of full recurrence
support from this monthly slice. No cleanup or extra production writes at closeout.

## C29-00 preflight checkpoint

The user approved this bounded session, including deletion of the new disposable
items, and requested focused commits. Private backup of the shared vdir/state and
normal configuration is complete. Fresh scoped sync selected only capture B with
forced deletion disabled; B contains zero resources after reconciliation and its
status is clean. There are no unresolved conflicts. All other collection bytes
and the normal configuration remain unchanged.

The previously documented legacy create journal entry for an absent unrelated
file remains byte-unchanged. It is not a current session write; no journal was
replayed, abandoned or removed to clear it. All other unsynced/prepared operations
remain stop conditions. The user confirmed readiness and the empty phone list. Only the two approved
monthly prerequisites were then seeded and uploaded; capture A remained untouched.

## C29-10 phone creation/conversion captured

The user completed the first phone actions and required that new evidence use
**only capture B**. All new captures and exported fixtures in this increment are
therefore from B; no A resource is included. Scoped sync remains B-only.

Synthetic captures are under `fixtures/phase0/v1/plan8-checklist-b/`: `000-empty`,
`002-seeded`, and `010-phone-create-convert`. After the phone round B contains
exactly three parents: phone-created monthly checklist `a`, converted monthly
checklist `b`, and unchanged monthly task `d`. All are open, date-only, due
2026-12-10 with `FREQ=MONTHLY`. There are no children yet. The conversion changes
JSON TaskType/uid while preserving recurrence metadata and series identity.
Only `a` was added and `b` changed; `d` is byte-unchanged. Privacy scan passes.
Other collection hashes and normal configuration remain unchanged. No capability
verdict is promoted from this phone baseline alone. Next: add/edit the two items
per baseline parent, then capture deletion separately before completion.

## C29-11 item creation and text edits captured

B-only `011-phone-items-and-text` contains seven resources: A/B checklist
parents, two open items (`z`,`q`) under each, and unchanged monthly task `d`.
Each item has the correct RELATED-TO and JSON parentuid link. Parent titles are
capitalized and notes are `x`; parent recurrence/identity JSON is unchanged from
010. Exactly four child resources were added and the two parents changed. There
are no removed resources, and `d`, other collection bytes and normal config are
unchanged. Privacy scan passes. Individual deletion is the next separate barrier;
no operation verdict is promoted by this baseline.

## C29-11D individual item deletion captured

B-only `011d-phone-delete-items` contains five resources. Exactly the two open
`q` children were removed, one from each open recurring checklist. Both parents,
both remaining `z` children and `d` are byte-unchanged from 011: the phone does
not rewrite parent counts or metadata for this shape. No resource/history was
created. Other collection bytes and normal configuration remain unchanged;
privacy scan passes. This establishes the narrow phone baseline for deleting
one of multiple open undated checklist items under an open monthly parent,
not last-item/completed-item/project-item deletion or Twydo acceptance.

## C29-12 baseline family completion captured

The user confirmed the parent stays open after the item tick. B-only
`012-phone-family-complete` contains nine resources: both live parents advance
December 10 → January 10, their original `z` items reopen without native dates,
and each family gains one completed parent and one completed linked child copy.
No deleted `q` returns or appears in history. Task `d`, other collection bytes
and normal configuration stay unchanged. Privacy scan passes. This completes
the first phone round; local replay/guard implementation precedes writer upload.

## Local candidate implementation and review

B-only baseline replay now covers direct creation, task conversion, parent title/notes,
item creation with inherited native due/mirror dates, individual deletion, manual
family completion/reset and history identities. Gates admit only the captured
monthly/date-only schedule and title/notes/type transition; destination edit rows
do not imply date/start/interval/project conversions. Item deletion has a dedicated
`recurring-checklist-item.delete` case rather than enabling parent deletion.

Failing regressions first exposed missing inherited dates and stale deletion
checks. Exact reset replay also exposed a history-parent identity bug on the
phone-converted B parent: its JSON uid differs from its CalDAV UID. The history
copy now gets its new identity, while the live parent keeps the internal identity
and copied items link to the new history parent. Focused fix: `56214d9`.

Independent review found two further safety issues, both reproduced before fixing:
parent/sibling holds were not durable across undo/redo/recovery, and item text
edits did not hold their parent. New optional journal read dependencies preserve
these bytes separately from mutations, and inverses retain them. Tests cover
removed parent on undo, removed sibling on redo, changed parent during recovery,
stale parent for ordinary/grouped text edits, and sync preserving a remote parent
without offering its stale held bytes as a write conflict. Old entries default
to no dependencies. No new capability verdict is enabled yet.

There is one explicit representation difference: the phone's reset A child has
PERCENT-COMPLETE:0, while the converted B child's reset omits that property. Both
are open. The regression asserts the difference rather than dropping it from the
comparison. The candidate retains the existing explicit-zero reopen writer;
subsequent phone execution on the candidate conversion remains required.

All 845 Rust tests, 49 Python tests, documentation links, clippy and formatting
pass after the review fixes. The exact 16-operation CLI writer sequence passes
on an offline B copy: 16 final resources, C/D due January 10 with open z items,
q deleted and excluded from history, all earlier baseline resources unchanged
except the intended d conversion. A private runner's initial expected-total
arithmetic was corrected from 15 to 16 before production use. No candidate
production write/upload has occurred at this checkpoint.

## C29-20 candidate writer and clean upload

A fresh B-only pull was byte-identical to the 012 phone checkpoint. The approved
16 CLI operations then created c directly, converted d, added x/q to each, changed
parent titles/notes and item titles, deleted only q, ticked z and manually completed
each parent. Candidate process configuration uses the shared vdir/state/lock with
after-edit sync disabled and baseline-only scratch rows; normal mode/configuration
stay verified and unchanged. No operation touched capture A.

B-only captures 020–026 retain the create/convert, item-text, deletion and reset
barriers for both writer branches. Exactly the intended one-resource edits/removals
and four-resource family resets occurred. B finishes with 16 resources: original
A/B baselines/history untouched, D converted in place, and C/D live z items with
one completed history pair each. C/D are open monthly checklists due January 10;
q is absent from live and history. Each journal entry's writes and durable read
dependencies stay inside B.

Scoped upload succeeds without conflicts; `027-twydo-upload` is byte-identical to
026, all 16 operation entries are confirmed synced, and all other collection bytes
and normal configuration remain unchanged. Privacy scan passes before importing
only these B snapshots. Phone inspection and note edit (C29-21), next family
execution (C29-22) and item edit (C29-23) remain pending. **No shipping capability
verdict is promoted from local execution/upload.**

## C29-21 phone accepts candidate creation/conversion and saves notes

The user confirmed inspection and note edits. B-only
`028-phone-writer-note-followup` contains the same 16 resources. Exactly C/D
parents change notes x → z and modification stamps. Their first phone save also
changes PRODID to 2Do and adds the three known sync image/audio/duration properties;
all four decorations and values are asserted explicitly in the regression.
Every JSON key, recurrence/date/identity value and creation field is preserved.
Every child and history resource, all earlier A/B branches inside B, and other
collections remain byte-unchanged. Both z items remain open with the candidate's
explicit PERCENT-COMPLETE:0; q stays absent. Privacy scan passes. No shipping
verdict is promoted yet; next manual completion and final item edit remain.
