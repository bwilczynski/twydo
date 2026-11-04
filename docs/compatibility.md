# Living alongside 2Do

Twydo is meant to be used together with 2Do on the iPhone: both apps edit the same tasks through
a CalDAV server. This page explains what that needs, how Twydo avoids writing anything the
iPhone could misread, and what has been checked.

## What you need

- **2Do on the iPhone, syncing over CalDAV.** 2Do's Dropbox sync is not supported: Twydo reads and
  writes iCalendar files, and 2Do uses that format only for CalDAV.
- **A CalDAV server that stores tasks (`VTODO`).** Testing used Synology Calendar; any server that
  keeps tasks as 2Do uploads them should work.
- **vdirsyncer**, to mirror the server into a local folder ([sync.md](sync.md)).

2Do for the Mac has not been tested (it needs a subscription that was not available). The iPhone
app is the reference for everything below.

## Moving from Dropbox to CalDAV

Switching 2Do's sync method replaces what the server holds, so do it carefully:

1. **Back up.** In 2Do, make a backup. If you use Dropbox sync, also copy the Dropbox folder
   `Apps/2Do` somewhere safe (copy it, never move it).
2. **Take an inventory** of the Dropbox data, so you can compare after the move:
   `python3 tools/decode_2do_dropbox.py <copy of Apps/2Do> inventory.2do.json`. The output holds
   your task data: keep it out of any repository.
3. **Create an empty task collection** on the CalDAV server.
4. **In 2Do, add the CalDAV account** and upload your data to it. Use *Replace Server Data* only
   when the destination collection is empty.
5. **Set up vdirsyncer and Twydo** ([sync.md](sync.md), [install.md](install.md)) and pull the
   collections.
6. **Compare**: `tdo export-json > export.json`, then
   `python3 tools/reconcile.py inventory.2do.json export.json` reports tasks and lists that do not
   match. Its output contains list names (and titles with `--show-titles`).

Then run `tdo check`: it reports the repeat kinds in your data, whether each is verified, and any
task Twydo cannot read.

## What Twydo keeps intact

Twydo changes only what an operation is about. A file it does not change is never rewritten; in a
file it changes, every unrelated line keeps its exact bytes, including properties and metadata
Twydo does not understand. Details: [data-model.md](data-model.md#what-twydo-preserves).

## Verified mode

Twydo writes a change only the way 2Do writes it: each kind of write is first made on the iPhone
and captured, so Twydo can write the same bytes (the `iphone_baseline` column). A write counts as
verified once Twydo's own result was also checked on the iPhone: read back correctly, and edited
again (the `twydo_to_iphone` and `iphone_to_twydo` columns). The results are recorded in
[`compat/matrix.toml`](../compat/matrix.toml), which is built into `tdo`. Many writes have a
baseline but are not verified yet.

By default (`[writes] mode = "verified"`), Twydo runs **only** writes the matrix marks as passing,
for the kind of task at hand. Anything else is refused before a file is touched, with the reason
on the status line (or on stderr from `tdo op`, which exits 1):

```
months-last-day repeats are not verified yet
reopen for yearly tasks is not verified yet (twydo_to_iphone)
nested projects and checklists are not verified yet
converting a project with items to a checklist is not supported by 2Do
```

"Not verified yet" means Twydo has not checked the write against the iPhone. "Not supported by
2Do" means the iPhone itself cannot do it (it greys out the type selector on a project with
items), so there is nothing to verify and the write stays refused in every mode, including `test`.

The part in parentheses names the direction that is missing: `twydo_to_iphone` (the iPhone has
not been checked reading Twydo's version) or `iphone_to_twydo` (Twydo has not been checked
reading the iPhone's). Reading is never refused: every task is shown.

To see what is allowed for one task: `tdo op check <uid>` prints each operation with `ok` or the
reason. To see it for your whole data: `tdo check`.

`[writes] mode = "test"` relaxes the check: it drops the `twydo_to_iphone` and `iphone_to_twydo`
columns of the operation rows and the checklist item-dates edge case (both apply only in verified
mode), but still needs the phone's baseline (`iphone_baseline`), the rule verdicts and the safety
rules, and still refuses the repeat kinds the iPhone misreads. It exists for testing against a
separate test account, and must never point at your real data: an unverified write can produce
something 2Do on the iPhone reads differently. See [config.md](config.md).

## What is verified today

The [editing-completeness audit and plan](superpowers/plans/2026-11-04-plan-8-editing-completeness.md)
tracks the remaining operations, imported-shape restrictions and later editing features. The
table below summarizes individual matrix cases, not every combination: a recurring project
also needs its own repeat's operation rows, for example, so its monthly/yearly repeat can still
block a move even though the recurring-project row passes. `tdo op check <uid>` gives the
effective result for an existing task.

The [Plan 8 phone session](reference/phase0/plan8-tui-completeness-session.md#regression-test-map)
finished its first batch: five logical synthetic cases, eight newly passing operation rows,
and narrowly guarded `until` execution. Its later project/item, lifecycle and native-date
batches remain unfinished. A passing row is combined with the shape restrictions below.

| Kind of task | Create | Ordinary edit | Complete | Reopen | Move | Delete |
|---|---|---|---|---|---|---|
| One-time task | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Monthly repeat (any day, Nth weekday) | ✅ | ✅ | ✅ | — | — | ✅ |
| Yearly repeat (29 February included) | ✅ | ✅ | ✅ | — | — | ✅ |
| Daily repeat | ✅ | ✅ | ✅ | — | — | — |
| Weekly repeat | ✅ | ✅ | ✅ | — | — | — |
| Repeat from completion | ✅ | — | ✅ | — | — | — |
| Checklist | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recurring checklist | — | — | Monthly only¹ | — | — | — |
| Project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recurring project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Repeating item of a project | — | ✅ | ✅ | ✅ | — | — |

“—” means not verified yet. Repeat changes use separate rows from ordinary edits. The complete
column also needs the rule/modifier's execution verdict and the gate's scheduling/family checks.

The newly enabled operations are:

| Matrix case / operation | Verified scope |
|---|---|
| `daily.repeat` | Set, remove or change between plain due-anchored daily/weekly/monthly/yearly rules, without ends or weekday selectors |
| `yearly.repeat` | Same scope; existing interval encodings are preserved |
| `checklist.repeat` | Set/remove plain date-only monthly/weekly recurrence on an existing checklist |
| `recurring-checklist.repeat` | Change/remove those monthly/weekly rules: interval 1, no end, start, alert, delay or completion anchor |
| `recurring-checklist.complete` | ¹Plain monthly day 1–28, interval 1, date-only, due anchored, no end/start/alert/delay; mixed or all-done items, no cancelled items |
| `ended-until-daily.reopen` | Reopen the final original of a top-level date-only interval-1 daily until series, without children/start/alerts/delay |
| `ended-until-monthly.reopen` | Same for plain monthly day 1–28 |
| `history-copy.reopen` | Top-level non-recurring, date-only task copies linked by RUID; no children/start/alerts/delay |

`until` completion now passes for top-level plain daily/monthly day 1–28 tasks with interval 1,
date-only due/end dates, no children/start/alerts/delay and a due-date anchor. Continuation,
inclusive boundary, final in-place completion and completion after reopening were checked both
ways. Other until combinations stay refused even though offset/base-rule evidence passes.
Count-ended originals, family history and other ended-series reopen shapes remain refused.

A recurring checklist's completion copies the family before resetting live items and removing
inherited dates. Item history keeps earlier individual completion times. Independent item starts
or due times, including start-only items without a due date, are refused without partial writes.
Weekly/yearly checklist completion, direct recurring-checklist creation and ordinary edits of a
recurring checklist remain unverified. A mixed card save needing such an edit is refused as a
whole and retains its text; a pure captured repeat change can pass on its own.

Existing repeat completion support remains: daily, weekly, weekday sets, monthly including the
29th–31st and Nth/last weekdays, yearly including February 29, from completion and count ends.
The [Plan 7 session](reference/phase0/plan7-capture.md) supplies those fixtures. Recurring projects
that end or repeat from completion remain refused. Moving/deleting one item, editing existing
from-completion tasks, project repeat changes, empty-project-to-checklist conversion and native
alert editing remain unverified. The [audit](superpowers/plans/2026-11-04-plan-8-editing-completeness.md)
tracks the remaining combinations; this batch does not certify the entire TUI surface.

The [RF/RA checklist](reference/phase0/plan8-rf-ra-checklist.md) now has phone
baselines for standalone monthly from-completion edits and monthly/weekly anchor
changes. Fixture and candidate TUI tests cover those shapes; Twydo-to-phone return
checks are pending. Their shipping operation rows remain unverified. A passing
custom matrix cannot widen these edits to daily/yearly rules, endings, families,
ordinary weekly/interval-2 field edits or custom alarms.

A pure repeat set/removal/reshape needs the repeat rows of both its source and destination
cases. Interval-only edits still need ordinary edit evidence; mixed saves need both. Newly
verified daily/yearly transitions do not establish changing count/date ends, completion anchors,
weekday sets or Nth rules. Their repeat writer preserves due/start/alerts bytewise, allowing
those already verified field representations to compose with a pure repeat edit; completion
has stricter shape restrictions because it advances dates and creates history.

Task/checklist/project and item creation and the previously verified type conversions remain
as documented in [tasks.md](tasks.md). `tdo op add` takes `--kind`, `--parent` and `--repeat`;
`tdo op edit` takes `--repeat`, `--no-repeat` and `--kind`. The TUI card exposes the same gate.
Known unsafe conversions and the three misread rule classes remain refused.

Three repeat kinds fail on the iPhone and Twydo never writes them in any mode: the last day of
the month, the last weekday and the Nth weekend day. 2Do 3.91 misreads them and its picker cannot
create them. Twydo still reads them, and the refusal says `… repeats are not supported by 2Do on
the iPhone`.

## How far this has been tested

One user's data, one iPhone (2Do 3.91 on iOS 27), one server (Synology Calendar), vdirsyncer
0.21.0 on Linux. Each verified row was tested both ways: the iPhone's change read by Twydo, and
Twydo's change read by the iPhone. The session logs are in
[reference/phase0/](reference/phase0/README.md), with synthetic test data in
[`fixtures/phase0/`](../fixtures/phase0/).

Verifying more needs a 2Do device and a test account. The method is in the
[phase-0 checklist](reference/phase0/README.md); `tools/seed_fixture.py` sets up test tasks on
the server so they do not have to be typed on the phone.
