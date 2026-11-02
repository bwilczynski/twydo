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
| `T` | monthly, due 00:00, start date, alert at due time (`start-due-alarm-offsets`) |
| `Y` | yearly, due 00:00, start date, alert at due time (`start-due-alarm-offsets`) |

All lists start in `Twydo test`. Dates are in the future except `L`.

| Round (step) | K (checklist) | P (project) | R (recurring project) | L | S | T | Y |
|---|---|---|---|---|---|---|---|
| 01-create | `K` checklist due 2026-12-10; items `k1` `k2` `k3` | `P` project, no date; `p1` due 2026-12-05; `p2` no date; `p3` due 2026-12-12 repeat monthly | `R` project due 2026-12-15 repeat monthly; `r1` due 2026-12-08; `r2`, `r3` no date | due 2026-08-15, repeat monthly | start 2026-12-01, due 2026-12-05, repeat monthly | start 2026-12-14, due 2026-12-20 00:00, alert at due time, repeat monthly | start 2027-02-13, due 2027-02-28 00:00, alert at due time, repeat yearly |
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
| H1 | A checklist parent has `TaskType: 1`, a project `TaskType: 2`; children carry `RELATED-TO:<parent UID>` and `parentuid` in `<2Do Meta>`; nothing else differs from a plain task | | |
| H2 | Checklist items have no `DUE` of their own; changing the checklist's date rewrites only the parent file | | |
| H3 | Editing, completing or reopening a child (project or checklist) changes only the child's file, the same way as a plain task (Plan 4 H1, H3, H4) | | |
| H4 | Completing a repeating child of a non-recurring project (`p3`) behaves like a plain monthly completion: the live child advances, a history copy is added, and both keep `RELATED-TO:<P>` | | |
| H5 | Completing a parent with open children asks for confirmation on the phone; the open children's files do not change | | |
| H6 | Completing a recurring project advances the live parent and its children's dates by one interval, reopens its completed children, and adds a completed copy of the parent plus completed copies of all children that point (`RELATED-TO`) at the copy | | |
| H7 | Reopening the completed copy of a recurring project turns it into an ordinary open project (no `RRULE`); its children's copies do not change; the live `R` is not touched | | |
| H8 | Moving a parent deletes and recreates the parent **and every child** with new UIDs in the destination, and rewrites the children's `RELATED-TO` to the new parent UID | | |
| H9 | Deleting a parent deletes its children's files in the same round; deleting a live repeat leaves its history copies | | |
| H10 | Completing a missed monthly occurrence (`L`, due 2026-08-15) advances by one interval only (2026-09-15), even though that date is also past; completing again gives 2026-10-15 | | |
| H11 | Completing a monthly task with a start date moves `StartDate` by the same interval as `DUE` (Plan 4 round 12 saw Dec 8 → Jan 8) | | |
| H12 | Completing a timed task (00:00) keeps `duetime: 0`; the at-due alert is relative (`TRIGGER:PT0S`), so it moves with the due and its `VALARM` is unchanged; the history copy keeps a copy of the alert | | |
| H13 | Completing a yearly task with start and alert shifts due and start by one year, as in H11 and H12 | | |

## Finish (🤖, then 👤)

```sh
python3 tools/capture_fixture.py scan "$P0/staging"     # must exit 0
```

👤 Confirm the staged content is synthetic. 🤖 Then copy to `fixtures/phase0/v1/`, fill in the
table above and the observations below, add the `project` case to `compat/matrix.toml`, set
`iphone_baseline` for the cases and edge cases this session covers, and commit.

## Observations

(Filled in after the session.)
