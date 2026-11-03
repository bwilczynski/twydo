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
