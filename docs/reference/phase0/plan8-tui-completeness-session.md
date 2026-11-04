# Plan 8 iPhone session: finish the existing TUI write surface

Status: **batch 1 phone baselines captured; local implementation checkpoint; final Twydo →
phone verification pending**. Execution requires the recorded
user approval below; this document itself does not authorize access.

## 1. Outcome

After implementation and this session's final verification, every **currently exposed TUI
write action** should work on every supported, well-formed task shape, or have a specific,
evidence-backed 2Do limitation. A parsed rule that can be created but cannot subsequently be
edited/completed is an open gap, not success.

This covers `o`/`O`/`a`, the card's Title/List/Type/Due/Start/Repeat/Notes, `ct`/`cp`/`cc`, date
shortcuts, `x`/Space, `m`, `dd`, `u`/Ctrl-r and grouped card saves. Repeating checklists and
existing project items are explicitly included. See the
[audit](../../superpowers/plans/2026-11-04-plan-8-editing-completeness.md).

It does **not** add the unbuilt alert editor, indent/outdent, paste, manual ordering, list
management, Project View, natural-language title extraction or notifier. Broken/ambiguous
resources, unsupported imported RRULEs and recurrence overrides remain protected. Existing
three misread repeat classes and unsafe type/interval changes remain refused unless new
device evidence actually overturns the specific finding.

## 2. Reuse, rather than repeat, the previous sessions

Use [Plan 7 results](plan7-capture.md), [Plan 6](plan6-baseline.md) and the
[gap session](gap-session.md) as baseline evidence where the exact operation matches:

- Plain task/checklist/project creation, items, ordinary parent completion/reset, ordinary
  field editing, and the captured type conversions are already established.
- Setting a weekly/monthly task repeat, changing weekly ↔ monthly, and interval changes are
  already covered. Setting a repeat on a **project** has Twydo → phone evidence; its phone
  baseline/read-back and later changes are the missing parts.
- Daily/weekly/monthly/Nth-weekday/yearly/from-completion completion, month-end and leap-day
  clamping, and count countdown/final state are captured for plain tasks.
- `until` has only the **ending** case. Repeating checklists have no recurring fixture.
- History reopening, moving and deletion must be matched to the exact top-level/family/item
  operation. A whole-family move is not evidence for moving one item alone.

Before seeding, produce a coverage ledger (section 4). Any action with sufficient existing
evidence is marked `reuse` with its fixture/test references; don't ask the user to redo it.

## 3. Safety and approval checkpoint

- [ ] Record the selected account, two collection IDs, explicit sync pair, local timezone,
  iPhone/2Do/server versions, binary revision and config/state locations **outside git**.
- [ ] Ask the user to approve the exact setup before backup, seed, sync or phone operations.
  Prefer the isolated test account **only if the phone is already safely using it**. Do not
  switch the phone's sync provider/account just for this session.
- [ ] If the phone still uses production, obtain fresh explicit permission for two new empty
  synthetic-only lists (`Twydo capture A`, `Twydo capture B`), backup, named sync pair,
  discovery, the exact write scope and cleanup. Past throwaway-list sessions are not permission.
- [ ] Back up the agreed vdir/state/config and make/confirm a restorable 2Do backup. Check for
  pending operations/conflicts before starting. Record unrelated collection hashes privately.
- [ ] Use a separate temporary config with after-edit/background sync disabled for the session
  process; coordinate any normal background writers before testing. Retain the existing
  vdir's shared lock/recovery identity. If temporary operation state would create a different
  lock, stop and agree an exclusive session arrangement first; separate configs must not allow
  two writers/sync runners to bypass each other's locks. Never replay a foreign journal.
  Do not change the user's normal production write mode, timers or config without consent.
- [ ] Positively allowlist the two new collection IDs; validate every seed/write/move UID
  against them. A UID collision, unexpected existing task, conflict or unrelated write stops
  the session. Do not run blanket `yes | discover` or remove arbitrary local collections.
- [ ] Keep all backups, exports, manifests with account details and scratch matrices outside
  the repository. Only approved synthetic fixtures may enter git.
- [ ] With separately approved read access, review the user's active rule/date/alarm shapes
  privately (no titles needed in the report). Add any phone-supported shape absent from the
  manifest before claiming readiness for their actual tasks; do not read production to do
  this during planning.

No server commands are supplied for blind execution: bind and validate the agreed targets
first. A live-account alternative cannot guarantee unrelated data stays unchanged if the
user edits it during the session; reconcile any difference, never restore it over their edits.

## 4. Evidence ledger and capture protocol

Prepare an external manifest **before the user starts tapping**. Each row has:

`case ID | collection | a UID | b UID | kind/parent | initial rule/dates/item states |
action | expected affected UID set | before capture | phone after | Twydo after |
phone follow-up | test name | matrix case/op/rule/edge | result/reuse reason`.

Use short synthetic titles `<ID>a` (phone baseline), `<ID>b` (Twydo twin); child suffixes
`-1`, `-2`, `-3`. UID mapping, not titles alone, identifies history and moved tasks. Include
initial resource/file counts, expected creations/deletions and parent links. Nothing below
is considered run until all these fields are populated. Seed just the current block to avoid
an enormous phone list. Let the user pause between blocks.

### Three checkpoints, not one long blind batch

1. **A — phone baselines:** seed supported plain starting states from existing fixtures using
   `tools/seed_fixture.py`; the phone performs previously unknown transformations. Capture
   every distinct intermediate state. An unsupported picker action is recorded, not simulated
   by injecting an assumed encoding.
2. **B — local implementation:** turn A's captures into regression tests, fix the writers,
   family planner and gate, and run lint/tests. The user need not remain at the phone. Save a
   restart manifest and resume with the same agreed lists; reseed fresh twins if needed.
3. **C — Twydo writes and phone follow-up:** run matching operations, inspect the synced result
   on the phone, then make a phone edit/completion on that result and capture its return.
   Finish with actual TUI verification in **verified mode** using the candidate matrix/build.

These can be separate appointments within this session plan. A is not enough to ship; a
fixed serializer's byte comparison is not a substitute for C's device check.

### Capture after every meaningful action

For **each substep**: record the before snapshot; one side acts; let it sync; capture A and B
collections; inspect export warnings/links/dates; record the result; only then continue. Never
set, change and remove a repeat before capturing the intermediate versions. Unrelated cases
may share a capture barrier, but each case's individual action remains recorded.

Use scenario IDs `plan8-tui-a` and `plan8-tui-b`, with identical zero-padded, monotonically
increasing steps, e.g. `010-rd-set-phone`, `011-rd-interval-phone`. Use the existing tool:

```sh
python3 tools/capture_fixture.py capture "$COLL_A" "$STAGING" \
  --scenario plan8-tui-a --step "$STEP" --device "$DEVICE" --action "$ACTION"
python3 tools/capture_fixture.py capture "$COLL_B" "$STAGING" \
  --scenario plan8-tui-b --step "$STEP" --device "$DEVICE" --action "$ACTION"
```

Variables must resolve to the approved synthetic collections and private staging directory.
Also preserve the pre-upload Twydo bytes/journal privately: a phone rewrite must not conceal
a faulty Twydo write. A deletion requires before/after inventory plus absence on the phone,
not an empty after-capture alone.

Compare all resource properties and JSON values, including UID/RUID/`uid`/`parentuid`,
RELATED-TO, RRULE, end/count fields, due/start/time, status/completion, alarms and unknown data.
Normalize only explicit twin identities, measured timestamps, property/JSON order and known
phone layout properties as in existing tests. Validate their meaning separately; never drop
all metadata, end keys, links or alarms from the comparison to make it pass. Recheck unchanged
Twydo resource bytes and stamps. Record same-day timing; from-completion twins must complete on
the same local day for a direct date comparison, otherwise compare at the phone's captured
instant in deterministic tests and separately verify actual Twydo dates at its own instant.

## 5. Block R — repeat editing and end behaviour

`D` is a recorded local date at least 14 days after the session starts. Explicit dates, not
relative phrases, go into the manifest. No device clock/timezone changes. For monthly/yearly
cases use known valid anchors from existing fixtures; preserve the same anchor in the twins.

| ID | Starting state and actions (capture each arrow/action) | Gap closed |
|---|---|---|
| RD | Plain task → daily → every 3 days → weekly → daily → none | Daily set/interval/class change/removal; phone baseline and Twydo return |
| RY | Plain → yearly → every 2 years → monthly → yearly → none | Yearly set/interval/class change/removal |
| RW | Plain → Mon+Thu → workdays → plain weekly → Mon+Thu → none | Changing weekday sets and plain/set-weekly transitions, not an unsupported multiweek interval |
| RN | Plain → 2nd Tue → last Fri → monthly → 2nd Tue → none | Nth/last weekday changes/removal without changing its interval |
| RF | Existing monthly after-done task: title, notes set/clear, due/time set/change/remove-time, start set/change/clear, interval change; completion on a separate twin branch | Editing existing from-completion tasks, preserved anchor and offsets |
| RA | Monthly from due → monthly after done → weekly after done → weekly from due → none; on a fresh plain pair set from-completion directly then remove | Both anchor switches, class change and direct set/remove |
| RE | Separate daily, weekly, monthly, yearly pairs: never → count 3 → until → never; then remove repeat on a branch with count 2 | End changes on existing rules; remaining versus original count preservation; no stale end fields |
| U1 | Daily due D, until D+2: complete three times, capturing D+1, D+2, then ended state | Continuation/history, inclusive end boundary and final state |
| U2 | Monthly due 2027-01-07, until 2027-03-07: complete to Feb 7, onto Mar 7, then finish; move this schedule forward if the session date requires it | Cross-check daily result against monthly encoding; no inferred history end fields |
| N5 | Fifth Tuesday due 2027-03-30 (April has no fifth Tuesday); complete twice, recording actual outcomes rather than assuming the next date | Skip/clamp/ordinal semantics; if the phone only offers “last”, record that difference instead of treating it as fifth |
| DR | Separate daily/monthly/yearly/from-completion branches: clear Due while keeping repeat; another clears Due and repeat together | Whether date removal is allowed, rejected or auto-filled; safe card behaviour |

RF's edits are separate captures so field-specific metadata changes are attributable. Reuse
existing date/notes evidence only when the resulting from-completion resource is proved to
use the same writer and preserves its anchor. For U2 choose and record exact dates by calendar
arithmetic, not elapsed 30-day months. Include one timed U1 branch with a start date and the
automatic due alert. Do not generalize that to absolute alerts or zoned recurrence.

## 6. Block F — checklists, projects and items

Prepare parent families with three items: one open, one already completed at a distinct time,
and one open. For projects give items independent dates (one undated); checklists carry only
the parent's scheduling. Keep cancelled-item testing separate if cancellation is available on
the phone; do not fabricate cancellation as though phone support had been observed.

| ID | Phone actions / matching Twydo actions | Required observations |
|---|---|---|
| FC | On a plain checklist with items, set monthly; edit parent title/notes/due/start/time, change interval, complete parent once; tick all live items and confirm parent stays open; tick parent again | Actual checklist repeat encoding, inherited dates, history identities/links, reset and completion times, manual final tick |
| FCR | Fresh recurring checklist branches: change monthly → weekly, remove repeat; add/edit/delete an item; complete/reopen an item while parent open | Checklist repeat row and full everyday item editing, no independent item recurrence |
| FCC | Separate branches: repeating task → checklist/project; empty recurring checklist → task; recurring checklist → project; empty recurring project → checklist/task | Conversions with recurrence retained, not inferred from non-recurring conversions |
| FE | Empty project → checklist, then phone edits Twydo's conversion | Missing baseline/read-back, reusing earlier Twydo display evidence |
| FP | Existing non-recurring project with items: set monthly → every 2 months → weekly → none | Missing phone baseline and changed/removed project-repeat resources |
| FI | Non-recurring project: create a new repeating child; on a different existing child set weekly → monthly → none | Fresh child creation versus editing, parent unchanged; complete repeating child independently and inspect its history |
| FR | Recurring project with an independently repeating child: complete child, capture; complete parent, capture | Whether supported, no double advancement, child reset/date/rule/history behaviour |
| FA | Checklist and project, each repeating from completion, with mixed items; complete once with start later than completion day | Parent anchor and child reset/date offsets, family copies |
| FN | Checklist and project, each with count 2; complete twice | Continuing then final family state, count fields in live/history and children |
| FU | Checklist and project, each daily until D+2 from D; complete three times | Family continuation, exact end and final completion, child history/end treatment |
| FD | Checklist whose items have inherited date copies: separate branches clear Due, set timed Due, set Start, clear Start | Exact cleanup of copied item dates without discarding unrelated data |

For FC create an additional recurring checklist directly in Twydo, add its items afterwards,
and let the phone edit/complete it. This establishes **creation**, not only setting recurrence
on an existing checklist. Repeat this direct-create check for a recurring project with an end
and one from completion; compare against the captured family encodings. Add one weekly and
one yearly recurring-checklist completion smoke check; different rule advancement already has
plain-task fixtures, but the family reset must be checked on these representations too.

If the phone cannot perform an action, capture its disabled control/message and unchanged
resources. Do not turn a supported parent with a repeat into an unsupported nested parent.
Nested-family probing is deferred unless an existing TUI action actually creates that shape;
items cannot currently become parents, so it is not a prerequisite for this session's goal.

## 7. Block L — moves, deletion and reopening

These use both approved lists and Done. Maintain distinct **live-series**, **history-copy** and
**ended-series** IDs. Do not report “reopen verified” without saying which one was reopened.

| ID | Actions, on independent branches | Evidence required |
|---|---|---|
| LM | Move daily, weekly, monthly, yearly, from-completion live tasks A → B, with history already present | Old/new UID and RUID, history references/locations, due/start/alerts; phone changes moved task and completes it |
| LF | Move recurring checklist/project families, including a non-recurring project with a repeating child | Every item's new identity/link, no orphans; complete/edit at destination |
| LD | Delete daily/weekly/from-completion live tasks and a recurring checklist family | Intended resources absent, completed history retention matches phone; undo after sync restores correct identities |
| LI | Delete one checklist item and one project item; move an item from each kind to list B using list move | Parent counts/state, independent task/link/date semantics; no implicit reparenting feature assumed |
| LH | Reopen a completed history copy from daily/weekly/monthly/yearly/from-completion series, and a recurring family history parent | Does live original stay unchanged? Reopened copy dates/rule/RUID/children; complete it again |
| LE | Reopen final count and final until occurrence of task/checklist/project, then complete again | Repeat/end/count restoration, whether it restarts or ends, no invented history; distinguish from undo |
| LC | Reopen one item of a completed checklist/project; add item to completed parent | Parent status changes or refusal, item links; cancelled parent is separate if phone supports it |

Reuse ordinary family move/delete/reopen evidence; add only checks for newly supported repeat
representations. Do not infer a live-series move from reopening a history copy (copies have no
RRULE). Avoid enabling broad case-level matrix rows for standalone child operations unless
their specific writer and baseline have been verified.

## 8. Block I — native date/alert representations

This is **preservation and completion of existing alerts**, not a new TUI alert editor. The
phone prepares the native representations; Twydo must not invent them by adding arbitrary
RRULE/TZID/metadata to live resources. Reuse existing timed/relative-alarm fixtures first.

| ID | Phone preparation and actions | Twydo verification |
|---|---|---|
| IA | A repeating task with one absolute alert; another with multiple relative/absolute alerts, including start-relative if the phone offers it. Complete once, capture live/history | Title/notes/date edit preserves unrelated alarms; completion shifts/retains each trigger exactly as observed; phone edits/completes result |
| IT | Inspect native phone output for floating/UTC/TZID due time and start-delay forms relevant to the private inventory | Capture any distinct writer branch; implement/verify its ordinary card edits and completion, or record a precise unsupported finding. Do not generalize floating-time evidence to zoned recurrence |

If the phone cannot create an imported representation that the user's active inventory uses,
record that unresolved dependency explicitly. This session cannot honestly certify such tasks
from floating-time fixtures alone. Fixed BYMONTHDAY/BYMONTH variants likewise require a
specific native fixture and lifecycle if they occur in the approved inventory. No need to
test arbitrary RFC shapes that neither the phone nor current TUI creates and the user does not use.

## 9. Block T — exercise the actual TUI, not only `tdo op`

Run after fixes, with candidate **verified-mode** matrix/build. If the TUI cannot consume a
candidate matrix, build a local candidate binary; do not enable global test mode as a workaround.
Each row gets a resulting synthetic fixture plus a phone inspection/read-back where it writes.

| ID | TUI acceptance |
|---|---|
| T1 | `o` and `O` in a list; `o` in Today (due today); new task in Scheduled/Done (message identifies destination even if row is invisible). `Tab` to card, create plain/project/checklist/recurring checklist. Phone sees correct types/dates |
| T2 | `a`, Enter-chain, `o` on an item: add siblings under project/checklist; Esc cancels only unsaved input. Complete all items: parent remains open until its own tick |
| T3 | `i`/`r`: change title, notes via editor, due/start/time, rule and type; remove those fields where allowed. Checklist item exposes Title/Notes only. Save/cancel both picker and card |
| T4 | `ct`/`cp`/`cc`, including the newly verified empty conversion and recurring conversions; already-correct type is a no-op. Known unsafe conversions remain refused |
| T5 | `t`/`T`, `[`/`]`, `{`/`}` on one-time, from-completion and repeating checklist parents; keep time when applicable and clean inherited item dates correctly |
| T6 | `x`/Space on repeat, recurring checklist, ending/from-completion project, history copy and ended series; confirm open children, cancel confirmation, check history and next occurrence |
| T7 | `m` and card List: move a repeating task, whole recurring family, standalone item. Save title + repeat + destination together, one undo step |
| T8 | `dd`, `u`, Ctrl-r: individual item and recurring family, repeat completion, grouped save; verify undo/redo before sync and undo after sync on newly supported multi-resource shapes |
| T9 | Reject a mixed card save containing a known-unsupported change: nothing changes, typed text retained. Controlled phone edit while card open: stale save never overwrites it; inspect reload/retry. Cancel always writes nothing |

Already verified navigation/search/help/sync behaviour needs only a smoke check, not another
round of device baselines. Interrupted writes, redo/recovery, remote-edit undo conflicts,
absolute-alert preservation and unknown-property byte preservation get local automated tests;
do not crash the phone or inject failures into production to demonstrate these.

## 10. Coverage and enablement decision

The coverage ledger must map **every newly enabled effective operation** to the exact old/new
case, rule/modifier and family shape that its gate selects. In particular:

- Separate `create`, ordinary `edit`, `repeat` set/change/remove, `complete`, live move/delete,
  history reopen and ended-series reopen. Split coarse matrix cases if needed rather than
  claiming an entire row from one narrow example.
- Cover rule-shape writers: daily, plain weekly, weekday set, plain monthly, Nth/last weekday,
  plain yearly; modifiers: from completion, count, until; contexts: task, checklist, project,
  checklist item, non-recurring project's repeating item and recurring project's repeating item.
- Full Cartesian device testing is not required when identical writer paths and encodings are
  demonstrated. Any generalization needs a written equivalence argument plus local cross-product
  gate/store tests. Distinct end/family/identity branches above always need their own captures.
- Verify a lifecycle for every newly creatable combination: create → ordinary edit → change
  repeat → complete/history → move/delete → undo/redo, using branches for destructive steps.
  Existing valid fixtures may supply proven portions. A new combination with a remaining
  unverified lifecycle is still an explicit blocker, not silently included in “all works”.
- Run `tdo op check` for every live, item, history and ended-series fixture, and actual operations
  in gate/store/TUI tests. `tdo op check` alone does not test Repeat/Type changes or all writer
  shape refusals. Use representative Op::Save operations as well.
- Do not mark pass for an action whose output was changed by a later fix until the fixed output
  has completed C's phone round trip. Record unsupported findings with exact scope.

### Done checklist

- [ ] A baselines complete or marked `reuse`/phone-unsupported with a reason; no ambiguous gaps.
- [ ] B regression tests prove the captured encodings and family behaviour; lint/tests green.
- [ ] C checks the final fixed Twydo writes, not an older build; phone follow-ups captured.
- [ ] TUI block T passes in verified mode with no unexpected feature refusals on supported cases.
- [ ] Matrix, gate, writers, CLI/TUI tests, as-is docs and Plan 8 updated together.
- [ ] Results list **remaining restrictions explicitly**. If any supported existing TUI action
  still fails, do not declare TUI completeness; schedule its identified missing capture/fix.

## 11. Cleanup, privacy and results

Capture final synthetic inventories and journal first. With separate user confirmation, remove
only the two approved throwaway lists through the agreed mechanism; do not let an emptied-list
sync accidentally recreate them. Reconcile all unrelated resources against the private baseline,
investigate differences and preserve concurrent legitimate edits. Restore only approved temporary
session settings, resume coordinated normal writers, and confirm normal config remains verified.

Run `python3 tools/capture_fixture.py scan "$STAGING"`, obtain human confirmation of synthetic
content, and copy only those two scenarios into `fixtures/phase0/v1/`. Review the staged diff for
personal data. A scanner pass is necessary but not a guarantee that decoded metadata is private.

Record approval, environment, revisions, round IDs, raw fixture references, comparison exceptions,
phone observations, fixes, final phone rechecks, matrix verdicts and unexplained differences in
this page's Results section. Do not put account IDs, URLs, credentials or private backup paths here.

## Results

### Setup — 2026-11-04

- The user explicitly selected **Use throwaway production lists**, approving local backup,
  the existing production sync pair and tests scoped to two new synthetic-only lists. No
  account switch; normal write mode remains verified; ask again before deleting the lists.
- Read-only local preflight found one applied operation awaiting sync, no prepared operations,
  no unresolved recorded conflicts and no recorded sync failures. Its content was not printed
  or copied into the repository. Do not undo or discard it as part of session setup.
- Took a private, permission-restricted backup of the local vdir, Twydo state and configuration
  (including the pending operation), plus private baseline hashes. The archive is readable.
  Private locations are recorded only in the external session manifest.
- The user subsequently confirmed a fresh 2Do backup, both empty lists and completed phone
  sync. Normal preflight sync succeeded. Discovery listing was inspected with creation prompts
  declined; only the two approved local collection directories were created, then discovery and
  sync succeeded. Both identities/display names were verified and both lists were empty.
- The old pending record is a creation from the previous day whose local resource no longer
  exists, not a live edit waiting for upload. It remains unchanged; no journal entries were
  abandoned or discarded to make the preflight appear clean.
- Took a synchronized private baseline/backup, bound the two collection IDs in an external
  allowlist, built the worktree binary, and prepared batch 1 entirely offline in verified mode.
  The normal production configuration and shared lock identity remain unchanged.
- Device-version confirmation remains pending. The previous environment record is not silently
  treated as a new device-version confirmation.

### Batch 1 — prepared/uploaded, awaiting phone actions

Sixteen synthetic resources in `Twydo capture A`; `Twydo capture B` is empty. Cases are FC, RD,
RY, U1 and U2, each with `a` (phone) and `b` (later Twydo) twins. FC has three items per twin;
item 2 is already completed, items 1 and 3 open. Parent due dates are December 10. RD/RY are
plain tasks due December 7. U1 is daily, due November 18, until November 20 (2026); U2 is
monthly, due January 7, until March 7 (2027). These are synthetic dates, not real obligations.

The external per-case manifest records the UID mappings, initial states, expected affected
resources/new copies, planned captures, coverage targets and regression-test names. Seeding
checked all UIDs for collision, held the shared production vdir's exclusive write lock, and
validated list identities/emptiness before writing. Only the 16 allowlisted synthetic files
were added. Upload succeeded; unrelated resource bytes match the synchronized baseline.

Private staging has `plan8-tui-a` and `plan8-tui-b` steps `000-seed` (before upload) and
`001-after-upload`. These captures are not yet approved/committed fixtures and no phone
baseline verdicts have been promoted.

First requested phone actions (capture barrier after all five; no action on `b` twins):

| Task | Action | Observation to capture |
|---|---|---|
| FCa | Set monthly repeat; do not complete yet | Checklist repeat metadata/type and inherited child scheduling |
| RDa | Set daily repeat | Daily setting, preserving its due date |
| RYa | Set yearly repeat | Yearly setting, preserving its due date |
| U1a | Complete exactly once | Continuing until-series live resource and history end fields |
| U2a | Complete exactly once | Same for monthly continuation |

If the phone does not offer an action, stop that case and record the exact limitation; do not
convert its type or substitute another rule to force it. Await the user's observations before
syncing/capturing `010-first-phone-actions` and advancing any case again.

### 010 — first phone transitions captured

The user reported synced. Both collections were captured privately at
`010-first-phone-actions`. A has 18 resources (16 initial plus two completed history copies),
B is empty. All eight `b`-twin resources, including checklist items, are byte-identical to their
pre-phone versions; unrelated production resource bytes still match the synchronized baseline.

- **FCa:** `TaskType` stays **1 (checklist)**, `RRULE:FREQ=MONTHLY`, `RecurrenceType` 258,
  `RecurrenceValue` 1, `RecurrenceFrom` 1 and own RUID. Setting repeat did not change its items.
  This establishes the missing phone baseline for setting a checklist repeat, not yet its
  completion/reset/history behaviour or Twydo's implementation.
- **RDa / RYa:** daily/yearly rules with codes 256/259, value 1, from due date; due dates retained.
- **U1a:** due November 18 → 19; `UNTIL=20261120` and live end metadata retained.
- **U2a:** due January 7 → February 7; `UNTIL=20270307` and live end metadata retained.
- Both until history copies have no RRULE, repeat/end/count codes zero, no
  `RecurrenceEndDate`, and RUID linking to the original. No verdict is promoted yet: exact
  boundary/final behaviour and fixed Twydo → phone round trips remain to be captured.

Next capture barrier: `020-boundary-and-checklist-complete`. Complete FCa's parent once
(confirm open items if asked), change RDa to every 3 days and RYa to every 2 years, and complete
the live U1a/U2a once each to test advancing exactly onto their end dates. Keep all `b` twins
untouched. Capture before any further completion or repeat change.

### 020 — checklist family completion and inclusive boundary captured

The user reported synced. Private step `020-boundary-and-checklist-complete` contains 24
resources in A (six new copies), zero in B. `b` twins and unrelated production bytes remain
unchanged; no resources were removed.

- **FCa** advances December 10 → January 10, stays an open checklist, and creates a completed
  checklist parent plus three completed child copies with new identities/links. The previously
  completed item's copy keeps its earlier completion time; other copies take the parent's time.
  All three live items reopen, retaining their identities/parent links, but **DUE and DTSTART
  are removed**. History child copies keep December 10. This is a distinct checklist-cleanup
  branch: do not enable the existing recurring-project planner without implementing/testing it.
- **U1a / U2a** advance onto November 20 / March 7 **inclusive**, remain open with RRULE/end
  metadata intact, and create one completed occurrence copy each with end metadata cleared.
- **RDa / RYa** retain their due dates and become every 3 days / every 2 years, with interval
  and `RecurrenceValue` 3 / 2 respectively.
- The phone assigned nonempty RUIDs different from the exported UID when setting repeats on
  RDa/RYa in step 010, and preserved them in 020. Keep those exact values in captures and
  explicit identity mappings in tests; do not normalize every RUID to the resource's own UID.

Next barrier `030-end-series-and-items`: complete FCa's three live items individually but not
its parent (record whether the parent stays open); finish U1a/U2a by completing the live task
once more; change RDa to weekly and RYa to monthly. Do not reopen ended series yet.

### 030 — manual parent tick and final until completion captured

Private step `030-end-series-and-items` has 24 resources in A, zero in B; none created or
removed. All `b` twins and unrelated production bytes are unchanged.

- **FCa:** all three live items are now completed, each with its own completion timestamp;
  the recurring checklist parent remains **open** and due January 10. No automatic final tick.
- **U1a / U2a:** final original resources complete in place, retaining RRULE, current due
  (November 20 / March 7), RUID and all live end fields. No new history copy on final completion.
- **RDa / RYa:** weekly / monthly respectively, interval value 1 and due dates retained.

Next barrier `040-reopen-ended-and-complete-parent`: manually complete FCa's parent, return
RDa/RYa to daily/yearly, and reopen **the ended original** U1a/U2a in Done (due November 20 /
March 7, not earlier history copies). Do not complete either reopened original again until
its reopen state has been captured. Report a refusal/unavailable phone action rather than
reopening a different copy.

### 040 — ended-series reopen and all-done parent completion captured

Private step `040-reopen-ended-and-complete-parent` has 28 resources in A (four new checklist
history-family resources), zero in B. No removals; `b` twins and unrelated bytes unchanged.

- **U1a / U2a:** the ended original resources reopen in place: status becomes NEEDS-ACTION,
  COMPLETED disappears, due/RRULE/RUID/end metadata stay unchanged, and no copy is created.
  This establishes the phone baseline for **until-ended daily/monthly** series reopening;
  count-ended series and recurring-family ending/reopening still need their own evidence.
- **FCa:** the explicit parent tick advances January 10 → February 10. Four completed history
  resources are created; each child's copy retains its individual item-completion timestamp
  from step 030. All live children reopen with no dates and unchanged links/identities.
- **RDa / RYa:** return to daily/yearly, retaining due dates and their existing RUIDs.

Next barrier `050-remove-repeat-and-refinish`: change live FCa monthly → weekly; remove RDa
and RYa's repeats; complete reopened original U1a/U2a once more to establish post-reopen
execution. Capture before any further action, including opening an earlier history copy.

### 050 — daily/yearly removal, checklist reshape and repeat execution after reopen

Private step `050-remove-repeat-and-refinish` has 28 resources in A, zero in B, with no
new/deleted resources. `b` twins and unrelated production bytes are unchanged.

- **RDa / RYa:** RRULE removed, `RecurrenceType` reset to 0, due dates retained. The phone
  leaves `RecurrenceValue` 1, `RecurrenceFrom` 1 and existing RUIDs in the non-recurring files.
  These stale-looking fields must be preserved, not cleaned up by an inferred normalization.
- **FCa:** monthly → weekly (`RecurrenceType` 257, value 1) without changing February 10 due
  or live item resources. This establishes a checklist rule-class change baseline.
- **U1a / U2a:** completing the reopened final originals ends them in place again, with updated
  completion timestamps and otherwise retained rules/end fields. No extra history resources.

Last baseline barrier for this first batch: `060-remove-checklist-repeat-and-history-reopen`.
Remove FCa's repeat. In Done reopen **only the first history copies** U1a due November 18 /
U2a due January 7, leaving the ended originals completed. Exact history UIDs are bound in the
external ledger. Leave RDa/RYa and all `b` twins alone. After capture, implement/test these
paths locally before asking for Twydo-twin verification; the remaining Plan 8 blocks are not
claimed complete by this batch.

### 060 — checklist repeat removal and earlier history reopening captured

The user confirmed **2Do 3.91** and reported synced. Private step
`060-remove-checklist-repeat-and-history-reopen` has 28 resources in A, zero in B. All `b`
twins and unrelated production bytes remain unchanged.

- **FCa:** repeat removed, type remains checklist, February 10 due and item states retained.
  `RecurrenceType` becomes 0; value/from/RUID remain, as for plain repeat removal.
- **Earlier U1a/U2a history copies:** reopen in place with no recurrence; completion state
  changes only, retaining due dates and RUID links. Both ended originals remain byte-unchanged.

### Local implementation checkpoint B

- All original uncommitted files were verified as this session's work; no user changes were
  discarded. Work remains in the dedicated Herdr worktree; the original checkout is clean.
- Privacy scan passed, and every captured SUMMARY matches the approved synthetic seed manifest.
  Both scenarios are copied into the worktree for tests, not staged/committed yet. No private
  manifests, production exports, backups, URLs or account identifiers were copied.
- `crates/twydo-core/tests/plan8_batch1.rs` compares all repeat transitions, until continuation /
  inclusive boundary / ending / reopen / re-execution, history-copy reopen and both checklist
  family completions against the phone. Modification stamps are asserted separately; creation
  time differences and the phone's private initial RUID are explicitly scoped, not discarded
  wholesale. Recurrence/end metadata, identity links, alarms and completion times are compared.
- Added recurring-checklist family planning: copy inherited dates to history first, then reopen
  live items and remove their inherited date copies. Tests cover one undo step, stable-identity
  redo, interrupted recovery and refusal without partial writes for uncaptured item shapes.
- Added narrowly scoped ended-until reopening for top-level date-only daily/plain-monthly
  series, using dedicated operation rows. Count/yearly/family/timed variants stay refused.
- Matrix entries now record phone baselines/read-back for the captured transitions, but their
  Twydo → phone verdicts remain **unverified**. `mod:until.execute` also remains unverified.
  Candidate matrices exist only in tests; none of these features is declared production-ready.
- Next: finish workspace validation, then perform the fixed binary's operations on the existing
  `b` twins under the approved collection allowlist. Capture each intermediate Twydo state and
  request actual iPhone read/edit/completion checks before any verdict promotion. Remaining
  Plan 8 batches, direct recurring-checklist creation, ordinary recurring-checklist field edits
  and broader family recurrence/lifecycle cases still need their planned evidence.

### 070/071 — first fixed-binary twin transitions, awaiting phone check

Workspace validation passed: `mise run test` (Rust workspace, 49 Python tests, documentation
links) and `mise run lint`. No new default verified-mode write verdicts were enabled.

With a private subprocess-only candidate config (same production vdir/state/lock, no sync table)
and a scratch matrix overriding only `mod:until.execute`, ran these allowlisted operations:
FCb repeat monthly, RDb daily, RYb yearly, and U1b/U2b completion once. Captured pre-upload at
`070-twydo-first-transitions`; all five live outputs and both history copies match step 010's
phone results under explicit twin/private-series-ID mappings, documented time differences and
known phone layout exclusions. A private inspection guard initially kept CRLF terminators in
decoded values and conservatively blocked upload; only the inspection was corrected, successful
task operations were not repeated, and the full target/non-target validation passed before upload.

Normal-config sync then succeeded; `071-twydo-first-upload` has 30 synthetic resources in A,
zero in B, with all `a` twins/unrelated resource bytes unchanged and normal mode still verified.
No production global settings were changed. Steps 070/071 remain privately staged for now.

Await phone inspection of the **b** twins and follow-up at `080-phone-check-twydo-first`:
FCb monthly/due Dec 10/items 1+3 open and 2 done, then add synthetic note `x`; RDb daily/due
Dec 7 then change to every 3 days; RYb yearly/due Dec 7 then every 2 years; U1b due Nov 19 and
U2b due Feb 7, then complete each once onto its until boundary. Keep `a` twins unchanged.
Do not promote verdicts merely because upload and local comparisons passed.


### 080 — first Twydo writes inspected and acted on by the phone

The user confirmed every requested b-twin inspection/action and reported synced. The normal
sync pair succeeded. Capture `080-phone-check-twydo-first` contains 32 resources in A (two
new history copies), zero in B. Exactly the five expected b-twin originals changed; all a
twins, other synthetic resources and unrelated production bytes remain unchanged.

- **FCb:** the phone read Twydo's monthly checklist and its mixed-state items; adding note
  `x` retains checklist type, December 10 due and monthly repeat.
- **RDb / RYb:** the phone read Twydo's daily/yearly series, then changed them to every three
  days / every two years. December 7 due and the Twydo-created series identities are retained.
- **U1b / U2b:** the phone completed Twydo's advanced originals onto November 20 / March 7
  inclusive. Both stay open with rule/end metadata intact and gain one completed history
  copy each, due November 19 / February 7 respectively.
- `plan8_batch1.rs` now compares the captured initial Twydo writes and phone follow-ups with
  the writers, including full recurrence/identity/end metadata. The 070 pre-upload and 071
  post-upload resources are also asserted byte-identical. Privacy scans passed before the
  synthetic captures entered the repository.

This is intermediate round-trip evidence, not completion of the planned transitions: rule
removal/reshaping, checklist family reset and ending/reopening still need their fixed-binary
phone checks. No matrix verdict is promoted at this checkpoint.


### 090/091 — fixed checklist reset and final until occurrences uploaded

After step 080, the user explicitly approved completing **FCb**, finishing **U1b/U2b**, and
syncing those synthetic results for phone inspection. The fixed binary ran only those three
operations, using the same private process-only candidate configuration and shared production
lock/journal identity. The scratch matrix overrides only `mod:until.execute`; the normal
configuration remains verified. No other operation or cleanup was authorized by this round.

Pre-upload `090-twydo-checklist-reset-and-until-end` and post-upload
`091-twydo-reset-and-end-upload` contain 36 resources in A, zero in B. Six live resources
changed and four checklist history resources were created. All a twins, other synthetic
resources and unrelated production bytes remain unchanged; upload preserved every resource's
bytes. Privacy scans passed before copying these synthetic captures into the repository.

- **FCb:** monthly live checklist advances December 10 → January 10, retains note `x`, and
  its three live items reopen with inherited DUE/DTSTART removed. The completed December 10
  checklist and three completed item copies have fresh identities/links; item 2's history
  retains its earlier completion timestamp.
- **U1b / U2b:** final originals complete in place, due November 20 / March 7, retaining RRULE
  and all end metadata. No additional until history copies are created.
- `plan8_batch1.rs` replays these operations through the store against the captured output,
  checks the 8/1/1-resource journal entries, and asserts exact upload preservation. Existing
  checklist tests cover undo/redo/recovery and refusal of uncaptured scheduling shapes.

Await actual phone inspection of the fixed checklist reset and final originals before claiming
this device round trip. Later phone follow-ups and Twydo-ended reopen still need their planned
checks. No operation/rule verdict is promoted merely because local validation/upload passed.
