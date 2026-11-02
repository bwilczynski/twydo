# Plan 6 baseline captures (iPhone): parents and real repeat shapes

Twydo blocks every task with a parent or children ("projects and checklists cannot be edited
yet"). It also blocks most repeat shapes the user's real tasks have. An audit of the phase-0 copy
(2026-11-02, shapes only) found:

| Shape in the real data | Count (open) | Gate reason today |
|---|---|---|
| non-recurring project, undated, with children (acceptance case P2) | 2 | projects |
| ↳ one child repeats monthly with a date | 1 | projects |
| monthly recurring project with 5 undated children (P1) | 1 | projects |
| monthly, date-only, occurrence already missed | most | `monthly-15th-three-months-late` |
| monthly, date-only, with a start date | 2 | `start-due-alarm-offsets` |
| monthly or yearly, due at 00:00, start date, alert at due time | 2 | due time, start date |
| monthly, due at 00:00, start date, no alert | 1 | due time, start date |
| checklist | 0 (the user may convert undated projects to checklists) | projects |

This session records what the iPhone writes for those shapes. Plan 6 is written from these
captures. It is the same method as [plan4-baseline.md](plan4-baseline.md).

- 👤 = you, on the iPhone (test 2Do, test CalDAV account). 🤖 = the agent, on the Linux desktop.
- **Test account only, synthetic titles only.** Never the production calendar or production iPhone data.
- Outputs stay outside the repository until the privacy scan passes and you confirm the content.

## Setup (🤖)

```sh
export P0=~/twydo-phase0
vdirsyncer sync twydo && vdirsyncer metasync twydo
export COLL="$P0/vdir/<Twydo test>" COLL2="$P0/vdir/<Twydo test 2>"
ls "$COLL" "$COLL2"     # both must be empty before round 01
```

## Rounds

Each round gives every task at most one action, so each file's change in a round's diff belongs
to one action. Parents and their children are separate files, so a round may act on a parent
and its children. After each round: 👤 let 2Do sync, 🤖 run `vdirsyncer sync twydo` and capture
both lists:

```sh
python3 tools/capture_fixture.py capture "$COLL"  "$P0/staging" --scenario plan6-list1 --step <NN-slug> --device iphone --action "<round actions>"
python3 tools/capture_fixture.py capture "$COLL2" "$P0/staging" --scenario plan6-list2 --step <NN-slug> --device iphone --action "<round actions>"
```

👤 Also note in one line what the phone *showed*: confirmation dialogs, what happened to the
children on screen, where the next occurrence landed.

| Task | Scenario (matrix case or edge case) |
|---|---|
| `K` + items `k1` `k2` `k3` | checklist (`checklist`) |
| `P` + children `p1` `p2` `p3` | non-recurring project (new case `project`); `p3` repeats (`project-recurring-child`) |
| `R` + children `r1` `r2` `r3` | monthly recurring project (`recurring-project`, `parent-mixed-children`) |
| `L` | monthly, missed occurrence (`monthly-15th-three-months-late`) |
| `S` | monthly, date-only, with a start date (`start-due-alarm-offsets`) |
| `T` | monthly, due 09:00, start date, alert at due time (`start-due-alarm-offsets`) |
| `Y` | yearly, due 09:00, start date, alert at due time (`start-due-alarm-offsets`) |

All lists start in `Twydo test`. Dates are in the future except `L`.

The real 00:00 tasks carry `duetime: 0` from the Dropbox era (05 §`duetime`); the iPhone's time
picker cannot set 00:00 (it jumps to 12:00), so `T` and `Y` use 09:00. Whether `duetime: 0`
behaves like any other time on completion is checked from the real data in Plan 6, not here.

| Round (step) | K (checklist) | P (project) | R (recurring project) | L | S | T | Y |
|---|---|---|---|---|---|---|---|
| 01-create | `K` checklist due 2026-12-10; items `k1` `k2` `k3` | `P` project, no date; `p1` due 2026-12-05; `p2` no date; `p3` due 2026-12-12 repeat monthly | `R` project due 2026-12-15 repeat monthly; `r1` due 2026-12-08; `r2`, `r3` no date | due 2026-08-15, repeat monthly | start 2026-12-01, due 2026-12-05, repeat monthly | start 2026-12-14, due 2026-12-20 09:00, alert at due time, repeat monthly | start 2027-02-13, due 2027-02-28 09:00, alert at due time, repeat yearly |
| 02-edit | `k1` → title `k1b`; `K` → date 2026-12-11 | `p1` → date 2026-12-06 | `r1` → notes `n` | — | — | — | — |
| 03-complete-child | complete `k1b` | complete `p1`; complete `p3` | complete `r1`; complete `r3` | complete | complete | complete | complete |
| 04-reopen-child | reopen `k1b` | reopen `p1` | — | complete again | — | — | — |
| 05-complete-parent | complete `K` (`k2`, `k3` open) | complete `P` (`p2` open) | complete `R` (`r2` open) | — | — | — | — |
| 06-reopen-parent | reopen `K` | reopen `P` | reopen the completed copy of `R` | — | — | — | — |
| 07-move | move `K` to `Twydo test 2` | move `P` to `Twydo test 2` | move the live `R` to `Twydo test 2` | — | — | — | — |
| 08-delete | delete `K` | delete `P` | delete the live `R` only | delete | delete | delete | delete |
| 09-delete-rest | — | anything of `P` left | `R`'s completed copies and anything left | completed copies | completed copies | completed copies | completed copies |

Round 08 deletes only the parents and the live tasks, so round 09's diff shows what 2Do left
behind (children of a deleted parent, history copies of a deleted repeat). After round 09 both
lists must be empty (vdirsyncer may need `--force-delete` per list).

## Hypotheses

Filled in after the captures: ✅ holds, ❌ does not (with what was seen), and the evidence step.
Any ❌ changes Plan 6 before code depends on it.

| # | Hypothesis | Result | Evidence |
|---|---|---|---|
| H1 | A checklist parent has `TaskType: 1`, a project `TaskType: 2`; children carry `RELATED-TO:<parent UID>` and `parentuid` in `<2Do Meta>`; nothing else differs from a plain task | ✅ (the parent also carries its own `uid` in the JSON; children have no `DUE` unless given one) | `fixtures/phase0/v1/plan6-list1/01-create` |
| H2 | Checklist items have no `DUE` of their own; changing the checklist's date rewrites only the parent file | ✅ | `fixtures/phase0/v1/plan6-list1/02-edit` (K) |
| H3 | Editing, completing or reopening a child (project or checklist) changes only the child's file, the same way as a plain task (Plan 4 H1, H3, H4) | ✅ (reopen leaves `PERCENT-COMPLETE:0`, as Plan 4 H4) | `fixtures/phase0/v1/plan6-list1/02-edit`, `03-complete-child`, `04-reopen-child` (k1, p1, r1, r3) |
| H4 | Completing a repeating child of a non-recurring project (`p3`) behaves like a plain monthly completion: the live child advances, a history copy is added, and both keep `RELATED-TO:<P>` | ✅ | `fixtures/phase0/v1/plan6-list1/03-complete-child` (p3: live `6a41…`, copy `905c…`) |
| H5 | Completing a parent with open children asks for confirmation on the phone; the open children's files do not change | ❌ the phone asks "This checklist still contains unfinished tasks…" (projects alike) and then **completes every open child** with the parent's `COMPLETED` time. A repeating child (`p3`) is completed with its `RRULE` kept: no advance, no history copy | `fixtures/phase0/v1/plan6-list1/05-complete-parent` (K, P) |
| H6 | Completing a recurring project advances the live parent and its children's dates by one interval, reopens its completed children, and adds a completed copy of the parent plus completed copies of all children that point (`RELATED-TO`) at the copy | ❌ partly: the live parent advances (UID kept) and a completed copy of the parent plus a completed copy of **every** child (new UIDs, `RELATED-TO` the copy) is added; live children are all reopened, but **their dates do not move** (`r1` stays Dec 8 under a Jan 15 parent). Copies of children completed earlier keep their own `COMPLETED`; the open child's copy takes the parent's | `fixtures/phase0/v1/plan6-list1/05-complete-parent` (R) |
| H7 | Reopening the completed copy of a recurring project turns it into an ordinary open project (no `RRULE`); its children's copies do not change; the live `R` is not touched | ❌ partly: the copy becomes an open project without `RRULE` (`RUID` kept) and the live `R` is untouched, but **every child copy is reopened too**, including ones completed before the parent. The same holds for any parent: reopening `P` also reopened `p3`'s history copy, leaving two open `p3` | `fixtures/phase0/v1/plan6-list1/06-reopen-parent` |
| H8 | Moving a parent deletes and recreates the parent **and every child** with new UIDs in the destination, and rewrites the children's `RELATED-TO` to the new parent UID | ✅ (moved files lose `PERCENT-COMPLETE:0`; the moved top-level repeat `R` gets its new UID in `RUID`, but the moved repeating child `p3` **keeps the old UID** in `RUID`) | `fixtures/phase0/v1/plan6-list1/07-move` + fixtures/phase0/v1/plan6-list2/07-move` |
| H9 | Deleting a parent deletes its children's files in the same round; deleting a live repeat leaves its history copies | ✅ (2Do shows its standard delete confirmation; deleting `P` also deleted `p3`'s history copy, which was a child of `P`) | `fixtures/phase0/v1/plan6-list2/08-delete`, fixtures/phase0/v1/plan6-list1/08-delete`, `09-delete-rest` |
| H10 | Completing a missed monthly occurrence (`L`, due 2026-08-15) advances by one interval only (2026-09-15), even though that date is also past; completing again gives 2026-10-15 | ✅ | `fixtures/phase0/v1/plan6-list1/03-complete-child`, `04-reopen-child` (L) |
| H11 | Completing a monthly task with a start date moves `StartDate` by the same interval as `DUE` (Plan 4 round 12 saw Dec 8 → Jan 8) | ✅ | `fixtures/phase0/v1/plan6-list1/03-complete-child` (S) |
| H12 | Completing a timed task keeps `duetime` (900); the at-due alert is relative (`TRIGGER:PT0S`), so it moves with the due and its `VALARM` is unchanged; the history copy keeps a copy of the alert | ✅ (the alert is 2Do's automatic one, `X-2DOAPP-ISAUTO:1`) | `fixtures/phase0/v1/plan6-list1/03-complete-child` (T) |
| H13 | Completing a yearly task with start and alert shifts due and start by one year, as in H11 and H12 | ✅ | `fixtures/phase0/v1/plan6-list1/03-complete-child` (Y) |

## Finish (🤖, then 👤)

```sh
python3 tools/capture_fixture.py scan "$P0/staging"     # must exit 0
```

👤 Confirm the staged content is synthetic. 🤖 Then copy to `fixtures/phase0/v1/`, fill in the
table above and the observations below, add the `project` case to `compat/matrix.toml`, set
`iphone_baseline` for the cases and edge cases this session covers, and commit.

## Observations

Session 2026-11-02, iPhone 2Do 3.91 against Synology Calendar, vdirsyncer 0.21.0 on Linux.

- **Round 01 needed two fixes before capture:** `Y` was first created monthly and `L` in
  December; both were corrected by edit, so `01-create` shows the intended shapes.
- **Parent completion and reopen act on the whole family.** Completing a parent with open
  children (after the confirmation) completes all of them; reopening a parent reopens all of
  them, whatever their state before. Spec §7 says completing a parent with unfinished children
  asks for confirmation; it does not say what happens to the children. Plan 6 must decide
  whether Twydo mirrors 2Do here (needed for the phone to show the same state) or not.
- **A repeating child completed through its parent stops repeating in practice** (`p3` in
  `05`): `STATUS:COMPLETED` with the `RRULE` still present. Reopening the parent brought it back
  (`06`).
- **Recurring projects do not shift their children's dates** (`05`). For the user's real
  recurring project (undated children) this does not matter; for acceptance case P1 (children
  with dates) it means a child's date falls behind the parent's after each completion.
- **Missed occurrences are never skipped** (`L`): each completion moves the date by one
  interval, so an overdue monthly task stays overdue until completed once per missed month.
- **The iPhone cannot set a 00:00 due time** (the picker jumps to 12:00). The real 00:00 tasks
  come from the Dropbox-era `duetime: 0`; `T` and `Y` used 09:00 instead.
- **vdirsyncer "completely emptied" stop** came up three times (round 00 for list 2, round 08
  for list 2, round 09 for list 1); each was synced with `--force-delete` for that list.
- **Not covered:** editing a recurring project itself or a repeating child, moving or deleting
  a single child, a repeating child inside a recurring project
  (`project-child-own-recurrence`), absolute alerts.

# Twydo → iPhone verification (Plan 6 Task 7)

Twydo now performs the family operations and the real repeat shapes (`tdo op`, and the TUI for
one confirmation), and the iPhone checks them. Three phone baselines are still missing, so the
phone captures those first. Then the phone edits Twydo-written tasks and Twydo reads them back.
A case/op passes when both directions look right. Same test account, same two lists, synthetic
single-letter titles; `Twydo test` = list 1, `Twydo test 2` = list 2. Both lists start empty.

## Setup (🤖)

```sh
cargo build -q -p tdo
tdo() { XDG_CONFIG_HOME=~/twydo-phase0/xdg "$PWD/target/debug/tdo" "$@"; }   # `test` write mode
uid() { tdo export-json | jq -r --arg s "$1" '.tasks[] | select(.summary==$s and .status=="open") | .uid'; }
```

After each round: 🤖 `vdirsyncer sync twydo` (with `--force-delete <list>` when a list empties),
capture both lists into scenarios `plan6-twydo-list1` / `plan6-twydo-list2` with `--device twydo`
(Twydo rounds) or `--device iphone` (phone rounds), and `tdo export-json` to check Twydo's view.
👤 checks each 🤖 round on the phone after letting 2Do sync.

Phone rounds 01–03 capture the three missing iPhone baselines. Before round 04 🤖 sets their
`iphone_baseline` in `compat/matrix.toml` (if they match 2Do's other encodings) and rebuilds, so
Twydo may edit those shapes in round 07.

## Rounds

| Round | Who | Actions | Check |
|---|---|---|---|
| 00-setup | 👤 | in list 1: checklist `C` due 2026-12-10 with `c1` `c2` `c3`; project `P` (no date) with `p1` due 2026-12-05, `p2` (no date), `p3` due 2026-12-12 repeat monthly; project `R` due 2026-12-15 repeat monthly with `r1` due 2026-12-08, `r2`, `r3` (no dates); `M` due 2026-09-15 repeat monthly; `S` start 2026-12-01, due 2026-12-05, repeat monthly; `T` start 2026-12-14, due 2026-12-20 09:00, alert at due time, repeat monthly; `Y` start 2027-02-13, due 2027-02-28 09:00, alert at due time, repeat yearly; `A` due 2026-12-22 10:00, alert 15 minutes before, repeat monthly | 🤖 `export-json`: all read, families linked |
| 01-phone-edit | 👤 | `R` → title `R2` (recurring project itself); `p3` → title `p3b` (repeating project item) | baseline: `recurring-project` edit, `project-recurring-child` edit |
| 02-phone-complete | 👤 | complete `p3b` | advances to 2027-01-12 with a history copy under `P` |
| 03-phone-reopen | 👤 | reopen `p3b`'s history copy (Dec 12) | baseline: `project-recurring-child` reopen |
| 04-complete-shapes | 🤖 | `tdo op complete` on `M`, `S`, `T`, `Y`, `A`, and on item `r1` | 👤 `M` due 2026-10-15; `S` Jan 5 / start Jan 1; `T` Jan 20 09:00 / start Jan 14, alert; `Y` 2028-02-28 09:00 / start 2028-02-13, alert; `A` Jan 22 10:00 with its 15-minute alert; `r1` completed; each repeat has one history copy |
| 05-complete-parents | 🤖 | TUI: select `C`, `x` (question shows), `x`; CLI: `tdo op complete <P> --all`; `tdo op complete <R2> --all` | 👤 `C` and `c1`–`c3` completed; `P`, `p1`, `p2` completed, live `p3b` completed with its repeat kept (not advanced); `R2` due 2027-01-15 with a completed copy holding copies of `r1`–`r3`, live `r1` reopened, `r2`, `r3` open, dates unchanged |
| 06-reopen-parents | 🤖 | reopen `C`, `P`, and `R2`'s completed copy | 👤 all items open again (including `p3b`'s history copies); the `R2` copy an open project without repeat; live `R2` unchanged |
| 07-edit | 🤖 | `R2` → title `R3`; `p3b` → notes `n`; `c1` → title `c1b` | 👤 the three changes show |
| 08-move | 🤖 | move `C`, `P` and the live `R3` to list 2 | 👤 the three families in list 2 with all items under the right parent; the open `R2` copy stays in list 1 |
| 09-phone-edit | 👤 | `c1b` → title `c1c`; complete `r2` (under the moved `R3`); `M` → notes `m` | 🤖 `export-json` shows all three, families still linked |
| 10-delete | 🤖 | delete `C`, `P`, the live `R3`, the open `R2` copy, and the live `M`, `S`, `T`, `Y`, `A` | 👤 those families and tasks gone, items with them; history copies remain |
| 11-delete-rest | 👤 | delete every remaining task (history copies) | both lists empty |

## Results

(Filled in after the session: per case/op, `twydo_to_iphone` and `iphone_to_twydo`, plus findings.)
