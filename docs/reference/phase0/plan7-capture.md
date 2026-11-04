# Plan 7 capture: repeat kinds and type conversions

Plan 7 (adding and reshaping tasks, [design](../../superpowers/specs/2026-11-03-tui-add-and-structure-design.md))
must write every 2Do repeat kind and every type conversion exactly as 2Do on the iPhone does.
Only monthly-on-a-day and yearly have been captured so far, and no conversion. This session
records what the phone writes. Same method as [gap-session.md](gap-session.md).

| What | Needed by | Round |
|---|---|---|
| each repeat kind (01 §6) set on a plain task: `RRULE` and the JSON codes | repeat parser, writing repeats (Plan 7 Tasks 2–4) | 01 |
| what 2Do's "Every Day" preset writes vs a custom "every 1 day" | `daily` in the repeat parser | 01 (`R01`, `R02`) |
| repeat modifiers: from completion, end after N, end on a date | same | 01 (`R14`–`R16`) |
| setting a repeat on a task with no due date | whether a save fills in Due | 02 |
| task → checklist, task → project, checklist → project, project → checklist, project with items → task, empty checklist → task | `ct`/`cp`/`cc` (Task 6) | 03, 04 |

New items under an existing parent are not captured again: `plan6-list1/01-create` already
holds items the phone created under a checklist and a project.

- 👤 = you, on the iPhone. 🤖 = the agent, on the Linux desktop.
- **A throwaway list in the production account, synthetic titles only.** Since the production
  migration the iPhone's 2Do syncs the production account, and switching accounts on the phone
  risks merging data. So the session ran in a new, empty list `Twydo capture` that the user
  created on the phone, after a full backup of the production vdir and state. Every write
  stayed in that list, and the list was deleted at the end.
- **No typing on the phone.** Round 00 seeds everything from the desktop; on the phone you only
  use the repeat picker, the type selector and delete.

## Setup (🤖)

```sh
# Back up first: the production vdir, state and config, after a fresh sync.
vdirsyncer sync twydo_prod
# 👤 creates the list `Twydo capture` on the phone; then pick it up here:
yes | vdirsyncer discover twydo_prod && vdirsyncer sync twydo_prod && vdirsyncer metasync twydo_prod
export F=fixtures/phase0/v1/plan6-twydo-list1/00-setup COLL=~/.local/share/twydo/vdir/<Twydo capture>
ls "$COLL"     # must be empty
cargo build -q -p tdo
tdo() { "$PWD/target/debug/tdo" "$@"; }   # production config, `verified` mode: plain creates only
L=$(basename "$COLL")
```

After each round: 🤖 `vdirsyncer sync twydo_prod`, then capture into scenario `plan7-list1` with
`--device twydo` (🤖 rounds) or `--device iphone` (👤 rounds), and `tdo export-json` to check
Twydo's view:

```sh
python3 tools/capture_fixture.py capture "$COLL" "$P0/staging" --scenario plan7-list1 --step <NN-slug> --device <d> --action "<round actions>"
```

## Rounds

Dates in 2026 unless given. Every repeat is set with the phone's repeat picker on the task
already there; the due date stays as seeded.

| Round | Who | Actions | Check |
|---|---|---|---|
| 00-seed | 🤖 then 👤 | 🤖 `tdo op add --list $L` plain tasks: `R01`–`R07`, `R14`–`R16` due Dec 7; `R08`, `R09`, `R11` due Dec 31; `R10` due Dec 8; `R12` due Dec 5; `R13` due 2028-02-29; `R17` and `K1`, `K2` without a due date. Then `seed_fixture.py "$F" "$COLL" --only C,c1,c2,c3,P,p1,p2,p3` and `seed_fixture.py "$F" "$COLL" --only P,p1,p2 --rename P=Q,p1=q1,p2=q2`, sync. 👤 let 2Do sync | 👤 20 plain tasks with those dates; checklist `C` (Dec 10) with `c1`–`c3`; project `P` with `p1` (Dec 5), `p2`, `p3` (Dec 12, monthly); project `Q` with `q1` (Dec 5), `q2` |
| 01-phone-repeat | 👤 | repeat picker: `R01` Every Day (preset) · `R02` custom every 1 day · `R03` every 3 days · `R04` Weekly · `R05` every 2 weeks · `R06` Mon and Thu · `R07` Work days · `R08` Monthly (on the 31st) · `R09` last day of the month · `R10` 2nd Tuesday of the month · `R11` last weekday of the month · `R12` 1st weekend day of the month · `R13` Yearly (29 Feb) · `R14` Monthly, repeat from completion date · `R15` Monthly, end after 3 · `R16` Monthly, end on 2027-06-30. Say in one line if the picker lacks any of these | 🤖 `export-json`: each task repeats with the expected class (01 §6), no warnings; record each `RRULE` and JSON codes |
| 02-phone-repeat-no-due | 👤 | `R17` (no due date): Weekly | 🤖 does `R17` now have a due date, and which? |
| 03-phone-convert | 👤 | type selector: `K1` → Checklist · `K2` → Project · `C` → Project · `P` → Checklist (say what the warning says) · `Q` → Task (say what the dialog says and where `q1`, `q2` end up) | 🤖 the changed lines of each file, item dates after `P` → checklist, `q1`/`q2` after `Q` → task |
| 04-phone-convert-empty | 👤 | `K1` (an empty checklist now) → Task | 🤖 `K1` back to a plain task; changed lines |
| 03b-phone-item-under-converted | 👤 | add item `y` to `K2` (added during the session, see Results) | 🤖 how `y` links to `K2` |
| 05-delete-list | 👤 | delete the list `Twydo capture` with its tasks | 🤖 move the local copy aside, `vdirsyncer discover`, sync; the rest of production unchanged |

A phone round passes when `tdo export-json` shows it, with no warnings, blocked tasks or broken
links. Anything else is a finding to record, not a failure of the session.

## Finish (🤖, then 👤)

The captures were staged in `~/twydo-phase0/staging-plan7`.

```sh
python3 tools/capture_fixture.py scan "$P0/staging"     # must exit 0
```

👤 Confirm the staged content is synthetic. 🤖 Then copy to `fixtures/phase0/v1/`, fill in the
results below, set the matrix columns, and commit.

## Results

Session 2026-11-03, iPhone 2Do 3.91 against Synology Calendar, vdirsyncer 0.21.0 on Linux.
Fixtures in `fixtures/phase0/v1/plan7-list1/` (round numbers as above). Twydo read every round
with no warnings, nothing blocked and every family linked.

**Repeats** (01, 02). What the phone wrote, setting a repeat on a plain task:

| Task | Picker | `RRULE` | `RecurrenceType` / `Value` | Other JSON |
|---|---|---|---|---|
| R01 | Every Day (preset) | `FREQ=DAILY` | 256 / 1 | |
| R02 | every 1 day | `FREQ=DAILY` | 256 / 1 | |
| R03 | every 3 days | `FREQ=DAILY;INTERVAL=3` | 256 / 3 | |
| R04 | Weekly | `FREQ=WEEKLY` | 257 / 1 | |
| R05 | every 2 weeks | `FREQ=WEEKLY;INTERVAL=2` | 257 / 2 | |
| R06 | Mon and Thu | `FREQ=WEEKLY;BYDAY=MO,TH` | 72 / 1 | |
| R07 | Work days | `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR` | 124 / 1 | |
| R08 | Monthly (31st) | `FREQ=MONTHLY` | 258 / 1 | |
| R10 | 2nd Tuesday | `FREQ=MONTHLY;BYDAY=2TU` | 261 / 2 | |
| R13 | Yearly (29 Feb) | `FREQ=YEARLY` | 259 / 1 | |
| R14 | Monthly, from completion | `FREQ=MONTHLY` | 258 / 1 | `RecurrenceFrom` 2 |
| R15 | Monthly, end after 3 | `FREQ=MONTHLY;COUNT=3` | 258 / 1 | `RecurrenceEndType` 2, `RecurrenceEndRepetitions` and `…Orig` 3 |
| R16 | Monthly, end on 2027-06-30 | `FREQ=MONTHLY;UNTIL=20270630` | 258 / 1 | `RecurrenceEndType` 1, `RecurrenceEndDate` 1814356800 (2027-06-30 12:00 UTC) |
| R17 | Weekly, no due date | `FREQ=WEEKLY` | 257 / 1 | due and `DTSTART` set to **today** (date only) |

In every row `RecurrenceFrom` is 1 (2 when from completion) and `RUID` becomes the task's own
UID. `RRULE` goes right after `DUE`.

- **Every Day and "every 1 day" are the same rule** (`FREQ=DAILY`, 256). Twydo's `daily` writes it.
- **Not offered by the iPhone picker:** last day of the month (`R09`), last weekday (`R11`),
  1st weekend day (`R12`). It offers "last Monday … Sunday of the month" (the Nth-weekday rule)
  instead. Writing those three classes cannot be checked against a phone-written baseline.
- **Removing a repeat leaves traces:** `R09` (the user tried the picker, then left it at none)
  has no `RRULE`, `RecurrenceType` 0, but `RecurrenceValue` 6, `RecurrenceFrom` 1 and `RUID`
  set to its UID.
- **A repeat set without a due date** (02) makes the due date today (date only), with the
  mirrored `DTSTART` 2Do's CalDAV upload always writes.

**Conversions** (03, 03b, 04), type selector on the iPhone:

- **Task → checklist / project** (`K1`, `K2`): `TaskType` 0 → 1 / 2, and a JSON `uid` key is
  inserted after `RecurrenceEndRepetitionsOrig`, with the keys in the parent order
  (`TaskType` before `TaskDuration`). The phone rewrote the whole file in its own layout
  (`PRODID`, `X-2DOAPP-SYNC-*`), as it does for any task it edits that Twydo created.
- **The JSON `uid` is 2Do's internal id, not the task's UID**, for these Twydo-created tasks
  (phone-created parents have the two equal). An item the phone then added under `K2` (03b)
  has `RELATED-TO` = `K2`'s UID, and JSON `parentuid` = the internal id. Twydo links by
  `RELATED-TO`, so the family reads fine.
- **Checklist with items → project** (`C`): only the parent's `TaskType` 1 → 2. The items keep
  their bytes, dates included.
- **A project with items cannot be converted on the iPhone**, neither to a checklist (`P`) nor
  to a task (`Q`): the type selector is greyed out. Twydo refuses both (matrix `unsupported`).
- **Empty checklist → task** (`K1`, 04): `TaskType` 0, the JSON `uid` removed, keys back in the
  task order.

**Production afterwards.** Comparing the production vdir with the backup taken before the
session: the capture list is gone, and the only other changes are 7 files in one list that
2Do on the phone wrote at the start of the session (its own completions; `PRODID` 2Do,
`LAST-MODIFIED` before any Twydo write). `tdo check`: gate passed, no parse errors.

**Matrix.** New `[[operation]]` rows: `daily` and `weekly` (all ops; `iphone_baseline` `pass`
for `edit`), and op `convert` with cases `task-to-checklist`, `task-to-project`,
`checklist-to-project`, `empty-parent-to-task` (`iphone_baseline` `pass`),
`project-to-checklist` and `parent-with-items-to-task` (`unsupported`). Every Twydo column
stays `unverified` until Plan 7 Task 13.

# Session B: Twydo's writes on the iPhone (Plan 7 Task 13)

Session A recorded what the phone writes. This session checks the other direction: Twydo
writes each new shape, the phone shows it, then acts on it, and Twydo reads the result back.
It fills the `twydo_to_iphone` (and, where the phone acts, `iphone_to_twydo`) columns of the
new matrix rows, and settles the guesses Plan 7 made:

| Guess | Round |
|---|---|
| a parent Twydo makes gets JSON `uid` = its own UID; the phone adds items under it correctly | 01, 02, 03 |
| a repeat Twydo sets gets `RUID` = own UID; completing it on the phone links its history copy | 01, 03 |
| last-weekday writes `RecurrenceValue` -1 | 01 (`LW`) |
| removing a repeat leaves `RecurrenceValue` / `RecurrenceFrom` / `RUID` | 02 |
| switching an end-dated repeat to never removes `RecurrenceEndDate` | 02 |
| empty project → checklist is a `TaskType` 2 → 1 edit | 02 |

- 👤 = you, on the iPhone. 🤖 = the agent, on the Linux desktop.
- **A throwaway list in the production account**, as in session A: a fresh backup first, a new
  list `Twydo capture 2` that you create on the phone, every write inside it, the list deleted
  at the end, and a comparison of the rest of production with the backup.
- **Twydo writes in `test` mode**, so the gate lets through the shapes being verified. This
  uses a separate, temporary config: same production vdir, its own state directory, no
  `[sync]`, and every command names the capture list. Your normal config stays in
  `verified` mode. Sync is run by hand with the production pair.
- **No typing on the phone** except the list name and one item (`y`).

## Setup B (🤖)

```sh
vdirsyncer sync twydo_prod                      # then back up vdir, state and config
# 👤 creates the list `Twydo capture 2`; then:
yes | vdirsyncer discover twydo_prod && vdirsyncer sync twydo_prod && vdirsyncer metasync twydo_prod
export COLL=~/.local/share/twydo/vdir/<Twydo capture 2> L=<its id>
ls "$COLL"                                      # must be empty
export X=<scratchpad>/xdg-capture              # config.toml: vdir = production vdir, state_dir = scratch, [writes] mode = "test"
tdo() { XDG_CONFIG_HOME=$X "$PWD/target/debug/tdo" "$@"; }
```

After each round: 🤖 `vdirsyncer sync twydo_prod`, capture into scenario `plan7-twydo-list1`
(`--device twydo` or `iphone`), `tdo export-json` to compare with what the phone shows.

## Rounds B

Dates in 2026 unless given.

| Round | Who | Actions | Check |
|---|---|---|---|
| 01-twydo-create | 🤖 | project `P` with `p1` (due Dec 5), `p2`; checklist `C` with `c1`, `c2`; recurring project `RP` (due Dec 15, monthly) with `r1`; repeating tasks, due Dec 7 unless given: `D1` daily · `D3` every 3 days · `W1` weekly · `W2` every 2 weeks · `WD` mon thu · `WK` workdays · `M1` monthly · `M31` monthly (Dec 31) · `LD` last day (Dec 31) · `N2` 2nd tue (Dec 8) · `LW` last weekday (Dec 31) · `WE` 1st weekend day (Dec 5) · `Y1` yearly · `Y29` yearly (2028-02-29) · `FC` monthly after done · `CT` monthly 3 times · `UN` monthly until 2027-06-30; plain `K1`, `K2`, `S1`, `S2` (due Dec 7); empty project `E`; empty checklist `EC` | 👤 every family shows its items; each repeating task shows the rule its name says (say which ones the phone shows differently or not at all) |
| 02-twydo-reshape | 🤖 | `S1` repeat weekly · `S2` repeat monthly, then `S2` → every 2 months · `M1` remove repeat · `W1` → monthly · `UN` → monthly (no end) · `K1` → checklist · `K2` → project · `C` → project · `E` → checklist · `EC` → task · `P` repeat monthly (due Dec 20) | 👤 each shows its new rule or type; `P` is now a recurring project with its items |
| 03-phone-act | 👤 | add item `y` to `K2`; complete `M31`, `W1`, `FC`, `CT`, `LW` and `RP` (confirm) | 🤖 `y` linked to `K2` (`parentuid`); each completed task advanced with a history copy linked by `RUID`; `RP`'s copy holds `r1`'s copy |
| 04-twydo-complete | 🤖 | complete `D1`, `WK`, `N2`, `Y29`, `S1` | 👤 each moved to its next date, with a completed copy in Done |
| 05-delete-list | 👤 | delete the list `Twydo capture 2` with its tasks | 🤖 move the local copy aside, discover, sync; the rest of production unchanged |

A Twydo round passes when the phone shows what the Check column says; a phone round when
`tdo export-json` shows it with no warnings, blocked tasks or broken links. A mismatch is a
finding: record it, fix the encoding with a test from the capture, and leave that row
`unverified`.

## Results B

Session 2026-11-04, iPhone 2Do 3.91 (the last version; it can no longer be updated from the App
Store) against Synology Calendar, vdirsyncer 0.21.0 on Linux. Fixtures in
`fixtures/phase0/v1/plan7-twydo-list1/` (round numbers as above). Twydo read every round with
no warnings, nothing blocked and every family linked.

**How Twydo wrote.** `test` mode still enforces the rule verdicts and the phone baseline, so
the creates of round 01 that the matrix did not yet allow were run with `tdo op --matrix`
(the hidden test hook) and a scratch copy of the matrix with `unverified` read as `pass`. The
copy was never committed. The temporary config and state were removed after the session.

**01, Twydo creates.** The phone showed right: project `P` with its items (`p1` dated), checklist
`C` with its items, recurring project `RP` with `r1`, and the repeats daily, every 3 days,
weekly, every 2 weeks, Mon and Thu, work days, monthly (on the 7th and on the 31st), 2nd
Tuesday, yearly (also on 29 Feb), from completion, end after 3 and end on a date.

- **Three kinds the iPhone does not support:** last day of the month (`LD`) shows as plain
  monthly on the 31st, last weekday (`LW`) as "every workday", 1st weekend day (`WE`) as "every
  weekend day". Its picker cannot create them either (session A). They most likely come from a
  later 2Do version than the iPhone's 3.91. Matrix: rule `edit` = `fail` for
  `months-last-day`, `months-nth-workday`, `months-nth-weekend-day`; Twydo never writes them
  (it still reads them).

**02, Twydo reshapes.** The phone showed right: a repeat set on a plain task (`S1` weekly, `S2`
monthly), an interval change (`S2` every 2 months), a repeat removed (`M1`), a class change
(`W1` weekly → monthly), an end date dropped (`UN`), task → checklist (`K1`), task → project
(`K2`), checklist with items → project (`C`, items kept), empty project → checklist (`E`),
empty checklist → task (`EC`), and a repeat set on a project with items (`P`, now a recurring
project).

**03, the phone acts on Twydo's writes.** All read back right:

- An item the phone added under `K2` (a parent Twydo made, JSON `uid` = its own UID) has
  `RELATED-TO` and `parentuid` = `K2`'s UID. So Twydo's choice of JSON `uid` works.
- Completing `M31`, `W1`, `FC`, `CT` on the phone made a completed history copy whose `RUID` is
  the live task's UID (the value Twydo wrote), and advanced the live task. So Twydo's `RUID`
  choice works. `FC` (after done) moved to one month after the completion day; `CT` counted
  down (`COUNT=2`, `RecurrenceEndRepetitions` 2).
- Completing the recurring project `RP` (with confirmation) made a completed copy holding a
  copy of `r1`, and reopened the live `r1`, as in Plan 6.

**04, Twydo completes.** Not run: Twydo does not yet execute daily, weekly or Nth-weekday rules
(its completion handles monthly-on-a-day and yearly only), so the gate refused all five with
`repeat rule shape not verified`, and `Y29` with "next due date after 2028-02-29 would not
fall on day 29". Nothing was written. Executing those rules is a follow-up.

**Production afterwards.** The rest of production is byte-for-byte the backup taken before
the session (`diff -r` empty); `tdo check`: gate passed.

**Matrix.** `twydo_to_iphone` = `pass` for: `create` of checklist, project, recurring-project,
daily, weekly, monthly, yearly, from-completion; `repeat` of one-time, weekly, monthly, project,
recurring-project; `convert` task-to-checklist, task-to-project, checklist-to-project,
empty-project-to-checklist, empty-parent-to-task. `iphone_baseline` and `iphone_to_twydo` =
`pass` for `create` of daily, weekly, from-completion (the phone set the same repeat on a plain
task in session A, and a Twydo create equals a plain create plus that repeat), and for
from-completion `complete` (03). Rule `edit` = `pass` for days, weeks, weekdays-set,
months-day-29-31, months-nth-weekday, years-feb-29 and the from-completion, count and until
modifiers; `fail` for the three unsupported kinds. Still `unverified`: `repeat` of daily,
yearly, from-completion, checklist and project-recurring-child, and every `execute` verdict of
the new classes.

# Session C: completing and editing every repeat kind (Plan 7 Task 15)

Twydo can now compute the next date for every kind it creates (Plan 7 Task 14), but verified
mode refuses those completions until they match 2Do. This session records 2Do's own behaviour
and compares Twydo's with it, twin against twin: each kind is seeded twice, the phone completes
the `a` copy and Twydo the `b` copy. It also settles six cases Task 14 had to guess:

| To confirm | Twin |
|---|---|
| monthly on the 31st after a short month, and whether it keeps its day | `M31` (completed twice) |
| yearly on 29 February | `Y29` |
| the last occurrence of an "N times" series | `C1` |
| an "until" series whose next date is past the end | `UN` |
| from completion with a start date later than the completion day | `FS` |
| a due date that is not on its rule | `OR` |

Same setup as session B: a fresh backup, a throwaway list `Twydo capture 3` that you create on
the phone, Twydo writing in `test` mode through a temporary config and `tdo op --matrix` with a
scratch matrix copy (never committed), the list deleted at the end and production compared with
the backup. No typing on the phone except the list name.

## Rounds C

Dates in 2026 unless given. Each kind is seeded as `<name>a` and `<name>b`.

| Round | Who | Actions | Check |
|---|---|---|---|
| 00-seed | 🤖 | twins: `D1` daily · `D3` every 3 days · `W1` weekly · `W2` every 2 weeks · `WD` mon thu · `WK` workdays · `N2` 2nd tue (Dec 8) · `LF` last fri (Dec 25) · `M31` monthly (2027-01-31) · `Y29` yearly (2028-02-29) · `FC` monthly after done · `FS` monthly after done, start Dec 20 · `C1` monthly 1 times · `UN` monthly until Dec 20 · `OR` mon thu due Wed Dec 9; due Dec 7 unless given. Singles for edits: `E1` daily, `E2` weekly, `E3` weekly, `E4` weekly (Dec 7) | 👤 let 2Do sync; 30 + 4 tasks |
| 01-phone-complete | 👤 | complete every `a` task once; complete `M31a` a second time (from its new date) | 🤖 each `a`: its next date, history copy, end fields |
| 02-twydo-complete | 🤖 | complete every `b` task once, `M31b` twice | compare each `b` with its `a`, byte for byte (except UIDs and stamps); 👤 the `b` tasks look like the `a` tasks |
| 03-twydo-edit | 🤖 | `E1` due +1 day and title `E1x` · `E2` every 2 weeks · `E3` weekly → monthly · `E4` remove repeat | 👤 each shows its change |
| 04-delete-list | 👤 | delete the list `Twydo capture 3` | 🤖 local copy aside, sync; production equal to the backup |

Where a `b` differs from its `a`, the phone's version wins: Twydo's rule is fixed with a test
from the capture, and that kind stays refused until it matches.

## Results C

Session 2026-11-04, iPhone 2Do 3.91, fixtures in `fixtures/phase0/v1/plan7-twydo-list2/` (an
extra step `04-phone-m31-again`: the phone's second completion of `M31a`, which round 01 missed;
the user also checked `E1x`–`E4` there). Twydo read every round with no warnings, nothing
blocked. Production afterwards was byte-for-byte the backup taken before the session.

**Twin comparison.** For each kind, a test completes the `b` twin as seeded (`00-seed`) with
Twydo's code, at the phone's stamp, and compares the live task and the history copy with the
phone's result for the `a` twin (`01-phone-complete`; `04-phone-m31-again` for `M31`'s second
completion). Left out: the stamps (`DTSTAMP`, `LAST-MODIFIED`, `COMPLETED`, checked to be Twydo's
`now`), `CREATED` (the twins were seeded seconds apart, and the phone gives a copy the time it
first received the task), the `a`/`b` UID (also in `RUID`) and title, and the phone's layout
lines (`PRODID`, `X-2DOAPP-SYNC-IMAGE`/`-AUDIO`, `X-2DOAPP-AUDIO-DURATION`). The phone also
rewrites the property order and the JSON key order of a file it changes; lines are compared as a
set and the JSON as data. Tests: `twin_*` in `crates/twydo-core/tests/edit_complete.rs`.

| Twin | Rule | Phone's result (`a`) | Twydo (`b`) |
|---|---|---|---|
| `D1` | daily | Dec 7 → Dec 8, copy | equal |
| `D3` | every 3 days | Dec 7 → Dec 10, copy | equal |
| `W1` | weekly | Dec 7 → Dec 14, copy | equal |
| `W2` | every 2 weeks | Dec 7 → Dec 21, copy | equal |
| `WD` | Mon, Thu | Mon Dec 7 → Thu Dec 10, copy | equal |
| `WK` | work days | Mon Dec 7 → Tue Dec 8, copy | equal |
| `OR` | Mon, Thu, due Wed Dec 9 | → Thu Dec 10, copy | equal |
| `N2` | 2nd Tuesday | Dec 8 → Jan 12, copy | equal |
| `LF` | last Friday | Dec 25 → Jan 29, copy; JSON `RecurrenceValue` -1 → **6** | differed, fixed |
| `M31` | monthly, due Jan 31 | → Feb 28 → Mar 28 (no memory of the 31st), a copy each | equal, both |
| `Y29` | yearly, due 2028-02-29 | → 2029-02-28, copy | equal |
| `FC` | monthly after done | due Dec 7, done Nov 4 → Dec 4, copy | equal |
| `FS` | monthly after done, start Dec 20 | due → Dec 4, start → Dec 17, copy | differed, fixed |
| `C1` | monthly, 1 time | completed in place: `RRULE`, due and JSON kept, no copy | equal |
| `UN` | monthly until Dec 20 | completed in place (Jan 7 is past the end), no copy | equal |

**Fixed in Task 15** (the phone wins):

- **From completion with a later start date** (`FS`): the phone counts from the completion day
  and ignores the start (due Dec 7 completed Nov 4 → Dec 4), and moves the start by as many
  days as the due (Dec 20 → Dec 17). Twydo counted from the start date (Jan 20); it now counts
  from the completion day, in `complete_recurring` and the gate alike.
- **"Last" in the Nth-weekday class** (`LF`): Twydo writes `RecurrenceValue` -1 for a last
  weekday (shown right by the phone, session B); the phone's own code is 6 (as in session A's
  `R09` trace), and it rewrote -1 to 6 on the live task when it completed `LFa`. Twydo's
  completion now writes 6 too, and accepts 6 when checking the metadata, so a last-weekday task
  the phone completed once can be completed in Twydo. Creating one still writes -1.
- Also from the Task 14 review: an `UNTIL` must name the UTC day of JSON `RecurrenceEndDate`
  (2Do writes noon UTC of the end day), or completing is refused; and the undo of a completion
  that ended a series in place counts as a repeat completion (the undo edge case applies), while
  reopening such a task is refused with a pointer to undo, as no capture shows what 2Do does.

**Not covered:** the phone's history copy of an "until" series that goes on (`UN` ended at
once); Twydo drops `RecurrenceEndDate` from the copy as `CT`'s copy drops the count (session B).
Reopening the last occurrence of an ended series. A recurring project's last occurrence.

**Edits** (`03-twydo-edit`, checked on the phone in `04-phone-m31-again`): `E1` due +1 day and
renamed `E1x`, `E2` every 2 weeks, `E3` weekly → monthly, `E4` repeat removed all showed right.

**Matrix.** Rule `execute` = `pass` for `days`, `weeks`, `weekdays-set`, `months-day-29-31`,
`months-nth-weekday`, `years-feb-29`, `mod:from-completion`, `mod:count` (with session B's `CT`
for counting down) and `mod:until`, each citing the twins above. Operation rows: `daily` and
`weekly` `complete` pass every column (the phone completed the `a` twins, Twydo read them back,
and Twydo's `b` result equals the phone's own completion; the user saw no difference between the
twins); `from-completion` `complete` gets `twydo_to_iphone` = `pass`; `daily` and `weekly`
`edit` get `twydo_to_iphone` = `pass` (`E1`, `E2`). The `weekly` and `monthly` `repeat` rows
already passed; their notes now cite `E3` and `E4`. In verified mode with the shipped matrix,
completing every kind Twydo creates is allowed, as is editing existing daily and weekly tasks,
weekly → monthly and removing a weekly repeat (`embedded_verified_completes_every_kind_session_c_verified`
in `crates/twydo-core/tests/gate.rs`). Still refused: completing the three kinds the phone
misreads, reshaping to daily, yearly or from completion, and editing an existing from-completion
task.
