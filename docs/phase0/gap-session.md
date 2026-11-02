# Gap session: verified mode for the real task shapes

On the phase-0 copy of the real data, verified mode still blocks writes the user needs, because
one Twydo column of the matrix row is missing. This session closes those rows on the iPhone
with the test account. Same method as [plan6-baseline.md](plan6-baseline.md).

| Row | Missing today | Real tasks it blocks | Closed by |
|---|---|---|---|
| project edit | `twydo_to_iphone`, `iphone_to_twydo` | the projects and their items (acceptance case P2) | 01, 02 |
| project complete | `iphone_to_twydo` | same | 04 |
| project-recurring-child complete | `twydo_to_iphone` | the monthly item inside a project | 01 |
| yearly edit, complete | `iphone_to_twydo` | yearly tasks | 02, 04 |

The phone acts in most rounds anyway, so the session also closes the cheaper `iphone_to_twydo`
gaps for reopen, move and delete on the same families. It also covers `project-recurring-child`
reopen in the Twydo → iPhone direction and captures the missing yearly reopen baseline:

| Row | Missing today | Round |
|---|---|---|
| project-recurring-child reopen | `twydo_to_iphone` | 03 |
| timed, start-date, alarms complete | `iphone_to_twydo` | 04 (`Y`) |
| checklist complete, reopen, move, delete | `iphone_to_twydo` | 04–07 |
| project reopen, move, delete | `iphone_to_twydo` | 05–07 |
| recurring-project reopen, move, delete | `iphone_to_twydo` | 05–07 |
| project-recurring-child move, delete | `iphone_to_twydo` | 06, 07 (with `P`) |
| monthly reopen | `iphone_to_twydo` | 05 |
| yearly reopen | `iphone_baseline`, `iphone_to_twydo` | 05 |
| yearly move, delete | `iphone_to_twydo` | 06, 07 |

- 👤 = you, on the iPhone (test 2Do, test CalDAV account). 🤖 = the agent, on the Linux desktop.
- **Test account only, synthetic single-letter titles only.** `Twydo test` = list 1,
  `Twydo test 2` = list 2. Both lists start empty.
- **No typing on the phone.** Round 00 seeds the data from the server; phone edits change dates
  with the picker; everything else is complete, reopen, move or delete.

## Setup (🤖)

The seed data is the iPhone-written setup round of the Plan 6 Task 7 session
(`fixtures/phase0/v1/plan6-twydo-list1/00-setup`). `tools/seed_fixture.py` copies those files
with fresh UIDs, rewrites the parent links (`RELATED-TO`, `parentuid`, `RUID`) and the
timestamps, and leaves everything else byte for byte as 2Do wrote it. The `P` family is
seeded twice; the second copy is renamed `Q`.

```sh
export P0=~/twydo-phase0 F=fixtures/phase0/v1/plan6-twydo-list1/00-setup
vdirsyncer sync twydo && vdirsyncer metasync twydo
export COLL="$P0/vdir/<Twydo test>" COLL2="$P0/vdir/<Twydo test 2>"
ls "$COLL" "$COLL2"     # both must be empty
cargo build -q -p tdo
tdo() { XDG_CONFIG_HOME=$P0/xdg "$PWD/target/debug/tdo" "$@"; }   # `test` write mode
uid() { tdo export-json | jq -r --arg s "$1" '.tasks[] | select(.summary==$s and .status=="open") | .uid'; }
```

After each round: 🤖 `vdirsyncer sync twydo` (with `--force-delete <list>` when a list
empties), capture both lists into scenarios `gap-list1` / `gap-list2` with `--device twydo`
(🤖 rounds) or `--device iphone` (👤 rounds), and `tdo export-json` to check Twydo's view.
👤 checks each 🤖 round on the phone after letting 2Do sync, and says in one line what the
phone showed (dialogs, where things landed).

```sh
python3 tools/capture_fixture.py capture "$COLL"  "$P0/staging" --scenario gap-list1 --step <NN-slug> --device <d> --action "<round actions>"
python3 tools/capture_fixture.py capture "$COLL2" "$P0/staging" --scenario gap-list2 --step <NN-slug> --device <d> --action "<round actions>"
```

## Rounds

| Round | Who | Actions | Check |
|---|---|---|---|
| 00-seed | 🤖 then 👤 | 🤖 `seed_fixture.py "$F" "$COLL" --only C,c1,c2,P,p1,p2,p3,R,r1,r2,M,Y` and `seed_fixture.py "$F" "$COLL" --only P,p1,p2 --rename P=Q,p1=q1,p2=q2`, then sync. 👤 let 2Do sync | 👤 list 1 shows checklist `C` (Dec 10) with `c1` `c2`; project `P` with `p1` (Dec 5), `p2`, `p3` (Dec 12, monthly); project `Q` with `q1` (Dec 5), `q2`; recurring project `R` (Dec 15, monthly) with `r1` (Dec 8), `r2`; `M` (Sep 15, monthly, overdue); `Y` (2027-02-28 09:00, yearly, start Feb 13, alert) |
| 01-twydo-edit | 🤖 | `tdo op edit <P> --title P2 --notes n`; `tdo op edit <p1> --due 2026-12-06`; `tdo op edit <Q> --due 2026-12-20`; `tdo op complete <p3>` | 👤 `P2` with note `n`; `p1` on Dec 6; `Q` on Dec 20; `p3` on 2027-01-12 with a completed copy (Dec 12) under `P2` |
| 02-phone-edit | 👤 | date picker only: `P2` → 2026-12-18; `p2` → 2026-12-07; `Q` → 2026-12-21; `Y` → 2027-03-01 (keep 09:00) | 🤖 `export-json` shows the four dates, families still linked, `Y` still yearly with its start and alert |
| 03-twydo-reopen | 🤖 | `tdo op reopen <p3's completed copy>` | 👤 two open `p3` under `P2` (Dec 12 without repeat, Jan 12 monthly) |
| 04-phone-complete | 👤 | complete `Q` (confirm), `C` (confirm), `R` (confirm), `Y`, `M` | 🤖 `Q`, `q1`, `q2` completed; `C`, `c1`, `c2` completed; `R` on 2027-01-15 with a completed copy holding copies of `r1` `r2`, live items open; `Y` one year on with start and alert moved, plus a history copy; `M` on 2026-10-15 plus a history copy |
| 05-phone-reopen | 👤 | reopen `Q`, `C`, `R`'s completed copy, `Y`'s history copy, `M`'s history copy | 🤖 `Q`, `C` and their items open; the `R` copy an open project without repeat, its items open, live `R` unchanged; `Y` and `M` copies open without repeat |
| 06-phone-move | 👤 | move `P2`, `Q`, `C`, the live `R` and the live `Y` to list 2 | 🤖 the four families in list 2 with their items (new UIDs, links rewritten); the `R` copy, `Y`'s copy and `M` stay in list 1 |
| 07-phone-delete | 👤 | delete `P2`, `Q`, `C`, the live `R`, the live `Y` (list 2) and the live `M` (list 1) | 🤖 those families and tasks gone with their items; list 1 keeps the `R` copy and the `Y` and `M` copies |
| 08-delete-rest | 👤 | delete everything left | both lists empty |

A Twydo round passes when the phone shows what the Check column says. A phone round passes when
`tdo export-json` shows it, with no warnings, blocked tasks or broken links.

## Finish (🤖, then 👤)

```sh
python3 tools/capture_fixture.py scan "$P0/staging"     # must exit 0
```

👤 Confirm the staged content is synthetic. 🤖 Then copy to `fixtures/phase0/v1/`, fill in the
results below, set the matrix columns for each row that passed, and commit.

## Results

Session 2026-11-02, iPhone 2Do 3.91 against Synology Calendar, vdirsyncer 0.21.0 on Linux.
Fixtures in `fixtures/phase0/v1/gap-list1/` and `gap-list2/` (round numbers as above). Every
round matched its Check column, on the phone and in `tdo export-json` (no warnings, nothing
blocked, families linked). Every row in the two tables above is now `pass` in
`compat/matrix.toml`, so verified mode allows project edit and complete, completing a repeating
project item, and yearly edit and complete.

- **Seeding works.** The phone pulled all 15 seeded tasks into the right families (00), so
  later sessions can set up data without typing on the phone.
- **Confirmations** on the phone appeared for `Q`, `C` and `R` (04: parents with open items),
  not for `Y` or `M`.
- **A phone date change on a project or an item** writes only `DUE` and `DTSTART` (02).
  **On a repeating task with a start date** (`Y`), it leaves `StartDate` where it was.
- **Completing moves `StartDate` by the due date's days, not by the interval** (04). `Y`'s due
  2027-03-01 → 2028-03-01 is 366 days (2028 is a leap year), and the start went
  2027-02-13 → 2028-02-14. Twydo moved the start by whole months, which would have given
  2028-02-13 (and a day off whenever the start and due months differ in length).
  **Fixed in this branch**: the start now moves by the same number of days as the due
  (`tests/edit_complete.rs`, `start_date_moves_by_the_due_dates_days`). A start on the 29th–31st
  is therefore no longer refused.
- **The history copy's `CREATED`** of a seeded task is the time the phone first received the
  task, not the file's `CREATED` (04, `Y`). This comes from seeding only; tasks the phone
  creates have the two equal.
- **Yearly reopen** (05, baseline) is the monthly one: `STATUS:NEEDS-ACTION`,
  `PERCENT-COMPLETE:0`, `COMPLETED` removed, nothing else.
- **Reopening a recurring project's copy** (05) reopened its item copies too, as in Plan 6
  (H7). **Moves** (06) recreated the families in list 2 with new UIDs and rewritten links, as
  in Plan 6 (H8). **Deleting a parent** (07) took its items; history copies in list 1 stayed.
- **vdirsyncer "completely emptied"** came up after 07 (list 2) and 08 (list 1); synced with
  `--force-delete`.
