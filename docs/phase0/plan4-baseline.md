# Plan 4 baseline captures (iPhone)

Plan 4 (writable core) writes task resources the way 2Do writes them. The phase-0 gate captured
only the monthly/yearly repeat fixtures, so this session records what the iPhone writes for the
other operations Plan 4 implements. Twydo's edits are built from these captures; any surprise
here changes the plan before code depends on it.

- 👤 = you, on the iPhone (test 2Do, test CalDAV account). 🤖 = the agent, on the Linux desktop.
- **Test account only, synthetic titles only.** Never the production calendar or production iPhone data.
- Outputs stay outside the repository until the privacy scan passes and you confirm the content.

## Setup (🤖)

```sh
export P0=~/twydo-phase0
vdirsyncer discover twydo && vdirsyncer sync twydo && vdirsyncer metasync twydo
ls "$P0/vdir"        # pick the test list (COLL) and a second test list for moves (COLL2)
export COLL="$P0/vdir/<test list>" COLL2="$P0/vdir/<second test list>"
```

Both test lists start empty; round 06 leaves them empty again.

## Rounds

To keep the phone work short, there is one task per scenario, named with a single letter.
The work runs in rounds: each round gives every task at most one action, so each file's change
in a round's diff belongs to exactly one action. After each round: 👤 let 2Do sync, 🤖 run
`vdirsyncer sync twydo` and capture both lists once:

```sh
python3 tools/capture_fixture.py capture "$COLL"  "$P0/staging" --scenario plan4-list1 --step <NN-slug> --device iphone --action "<round actions>"
python3 tools/capture_fixture.py capture "$COLL2" "$P0/staging" --scenario plan4-list2 --step <NN-slug> --device iphone --action "<round actions>"
```

| Task | Scenario (matrix case) |
|---|---|
| `A` | one-time: create, title, complete, reopen, move, delete |
| `B` | date-only, plus notes: create, notes, clear notes, change date, remove date, delete |
| `C` | timed: create, change time, make date-only, delete |
| `D` | start-date: create, change start, remove start, delete |
| `E` | monthly: create, notes, complete, reopen the history copy, move, delete |
| `F` | yearly: create, move, delete |

All dates are in the future. `COLL` = list `Twydo test`, `COLL2` = list `Twydo test 2`.

| Round (step) | A | B | C | D | E | F |
|---|---|---|---|---|---|---|
| 01-create | due 2026-12-03 | due 2026-12-03 | due 2026-12-03 17:30 | start 2026-12-01, due 2026-12-03 | due 2026-12-10, repeat monthly | due 2026-12-15, repeat yearly |
| 02-edit | title → `A2` | notes → `x, y` newline `z` | time → 18:45 | start → 2026-12-02 | notes → `n` | move to `Twydo test 2` |
| 03-complete | complete | clear notes | remove the time (keep the date) | remove the start date | complete | delete |
| 04-reopen | reopen | date → 2026-12-05 | delete | delete | reopen the completed copy | |
| 05-move | move to `Twydo test 2` | remove the due date | | | move the live task to `Twydo test 2` | |
| 06-delete | delete | delete | | | delete the live task and any completed copies | |

## Hypotheses

Filled in after the captures: ✅ holds, ❌ does not (with what was seen), and the evidence step.
Any ❌ stops Plan 4 until the plan is amended for it.

| # | Hypothesis | Result | Evidence |
|---|---|---|---|
| H1 | Edit title/notes changes only `SUMMARY`/`DESCRIPTION`, `DTSTAMP`, `LAST-MODIFIED` | ✅ (2Do also moves `DTSTAMP`/`LAST-MODIFIED` below `CREATED`; position is not meaningful) | `fixtures/phase0/v1/plan4-list1/02-edit` (A title, B notes) |
| H2 | Cleared notes are written as an empty `DESCRIPTION:` | ✅ | `fixtures/phase0/v1/plan4-list1/03-complete` (B) |
| H3 | Complete (one-time) sets `STATUS:COMPLETED`, adds `PERCENT-COMPLETE:100` and `COMPLETED:<UTC>`; nothing else beyond timestamps | ✅ | `fixtures/phase0/v1/plan4-list1/03-complete` (A2) |
| H4 | Reopen (one-time) sets `STATUS:NEEDS-ACTION` and removes `COMPLETED` and `PERCENT-COMPLETE` | ❌ reopen sets `STATUS:NEEDS-ACTION`, removes `COMPLETED`, and sets `PERCENT-COMPLETE:0` (the line stays) | `fixtures/phase0/v1/plan4-list1/04-reopen` (A2) |
| H5 | Move = same `UID` and content in the destination folder (only timestamps may change); source file removed | ❌ a move deletes the source and creates a **new resource with a new UID** (file `<new uid>.ics`) in the destination; content otherwise identical except `DTSTAMP`/`LAST-MODIFIED`, and a non-empty `RUID` becomes the new UID | `fixtures/phase0/v1/plan4-list1/05-move` + `fixtures/phase0/v1/plan4-list2/05-move` (A2, E); `fixtures/phase0/v1/plan4-list2/02-edit` (F) |
| H6 | Delete removes the file; no other resource changes | ✅ | `fixtures/phase0/v1/plan4-list1/04-reopen` (C, D), `fixtures/phase0/v1/plan4-list2/03-complete` (F) |
| H7 | Due edit changes `DUE` and its `DTSTART` copy together; removing the due date removes both | ✅ | `fixtures/phase0/v1/plan4-list1/04-reopen`, `05-move` (B) |
| H8 | A timed due is floating local time `DUE:YYYYMMDDTHHMMSS` (no `TZID`, no `Z`), and `DTSTART` copies it | ❌ partly: `DUE`/`DTSTART` are floating as assumed, but the JSON gains `"duetime": <HHMM int>` (appended last), and 2Do adds an automatic VALARM (`TRIGGER:PT0S`, `ACTION:AUDIO`, `DESCRIPTION:<title>`, `X-2DOAPP-SOUND`, `X-2DOAPP-ISAUTO:1`, `X-2DOAPP-RELDUETIME:1`). Changing the time updates `duetime` only; removing the time removes `duetime` and the auto VALARM | `fixtures/phase0/v1/plan4-list1/01-create`, `02-edit`, `03-complete` (C) |
| H9 | Start edit changes only `StartDate` in `<2Do Meta>` (epoch seconds of UTC midnight of the start day); removing it deletes the key | ✅ (`StartDate` sits right after `RecurrenceEndRepetitionsOrig` in the key order) | `fixtures/phase0/v1/plan4-list1/01-create`, `02-edit`, `03-complete` (D) |
| H10 | 2Do creates a plain task with the properties and metadata key order seen in `rule-*/01-create`, with `RecurrenceValue`/`RecurrenceType`/`RecurrenceFrom` = 0 and `RUID` = its own UID | ❌ partly: same layout and key order, recurrence fields 0, but a plain task has `RUID: ""`; only repeating tasks carry their own UID in `RUID` | `fixtures/phase0/v1/plan4-list1/01-create` (A, B vs E, F) |
| H11 | Completing a monthly task again matches `rule-months-day-1-28/02-complete` (advance live, add copy) | ✅ | `fixtures/phase0/v1/plan4-list1/03-complete` (E: live `44123…`, copy `7d03…`) |
| H12 | Moving or deleting a recurring task touches only its live resource, not its history copies | ✅ the history copy stays in the source list with `RUID` naming the old UID | `fixtures/phase0/v1/plan4-list1/05-move` (E) |

## Finish (🤖, then 👤)

```sh
python3 tools/capture_fixture.py scan "$P0/staging"     # must exit 0
```

👤 Confirm the staged content is synthetic. 🤖 Then copy to `fixtures/phase0/v1/`, fill in the
table above and the observations below, set `iphone_baseline` in `compat/matrix.toml`, and commit.

## Observations

Session 2026-11-01, iPhone 2Do 3.91 against Synology Calendar, vdirsyncer 0.21.0 on Linux.

- **Move** is delete + create with a fresh UID, for plain and repeating tasks. History copies
  of a moved repeating task stay behind and keep pointing (`RUID`) at the old UID.
- **Timed due dates** carry `duetime` (HHMM integer, e.g. 1730) in `<2Do Meta>`, and the phone
  added an automatic at-due-time alert (`X-2DOAPP-ISAUTO:1`). That alert probably comes from a
  2Do default-alert setting; Twydo does not create it (it notifies due tasks itself) but removes
  it with the time, as 2Do does. A 00:00 due time is `duetime: 0` (consistent with 05).
- **Reopening a repeat's history copy** turns it into an ordinary open task (no `RRULE` comes
  back, `RUID` unchanged); the live task is not touched. Same field changes as a one-time reopen.
- **New metadata keys are not appended blindly**: `StartDate` sits after
  `RecurrenceEndRepetitionsOrig`, `duetime` comes last.
- **vdirsyncer:** when the last task of a collection is deleted remotely, `vdirsyncer sync`
  stops with "Storage … was completely emptied" and needs `--force-delete` for that collection.
  The Plan 3 sync runner must handle this case.
