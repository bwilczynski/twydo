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

| Kind of task | Create | Edit | Complete | Reopen | Move | Delete |
|---|---|---|---|---|---|---|
| One-time task | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Monthly repeat (any day, Nth weekday) | ✅ | ✅ | ✅ | — | — | ✅ |
| Yearly repeat (29 February included) | ✅ | ✅ | ✅ | — | — | ✅ |
| Daily repeat | ✅ | ✅ | ✅ | — | — | — |
| Weekly repeat | ✅ | ✅ | ✅ | — | — | — |
| Repeat from completion | ✅ | — | ✅ | — | — | — |
| Checklist | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Recurring project | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Repeating item of a project | — | ✅ | ✅ | ✅ | — | — |

"—" means not verified yet. The Complete column is the matrix row, but the gate also needs
the repeat rule's own `execute` verdict. Since Plan 7 session C every repeat kind Twydo creates
passes it but an end date: daily, weekly, on set weekdays, monthly on any day (the 29th to 31st
included), the Nth or last weekday, yearly (29 February included), from completion, and repeats
with an end by count. A repeat with an end date (`mod:until`) stays unverified: only its last
occurrence was captured, not one that moves on. A recurring project whose repeat ends or counts
from completion is refused to complete in every mode (not captured for families). The phone
completed one copy of each kind and Twydo an identical copy, and
Twydo's result equals the phone's byte for byte (see
[`plan7-capture.md`](reference/phase0/plan7-capture.md), session C). For a repeating
task, "Edit" covers the due date, title and other fields, and changing the repeat's interval,
not setting, removing or reshaping the repeat (the `repeat` rows). Interval-only changes are
allowed within a class whose rule `edit` verdict passes and whose task row `edit` passes: daily,
weekly, monthly and yearly tasks. The same interval encoding is accepted as verified for the
29th to 31st, 29 February, and repeats with an end, so those can change interval when their
task's `edit` row passes. An Nth-weekday repeat's interval is never changed: 2Do keeps its
ordinal where the interval would go. Nor does a repeat on set weekdays get one, in any mode:
Twydo completes it every week only. Editing an existing "from completion" task stays refused
(its `edit` row has no baseline). The gate also covers creating items, projects, checklists and
repeats, setting and removing a repeat, and converting a task's type (`convert` cases for task to
checklist, task to project, checklist to project and empty parent to task; `daily` and `weekly`
repeat classes). The iPhone baselines for these were captured and Twydo's writes were checked
on the phone (see [`plan7-capture.md`](reference/phase0/plan7-capture.md)). In verified mode that
allows **creating** checklists, projects, recurring projects and items, and creating daily,
weekly, monthly, yearly and from-completion repeating tasks, and **converting** a task to a
checklist or project, a checklist to a project, and an empty checklist or project to a task.
**Setting a weekly or monthly repeat on a plain task, removing a weekly or monthly repeat, and
changing a weekly repeat to monthly or a monthly one to weekly are allowed** (session A: the
phone's own repeats; sessions B and C: Twydo's writes shown right). Plain daily/yearly repeat set/change/removal now also passes, restricted to due-anchored rules without ends or weekday selectors. From-completion and project repeat changes remain unverified.
Empty project to checklist lacks its iPhone baseline. `tdo op add` creates plain tasks by default; `--kind task|project|checklist`,
`--parent UID` (an item under a checklist or project) and `--repeat TEXT` (a rule in words such as `weekly`, `2w`,
`mon thu`; without `--due` the task is due today, as in 2Do) create the other shapes, and
`tdo op edit UID` takes `--repeat TEXT`, `--no-repeat` and `--kind` to reshape a task (`--kind` naming the type the file already has writes nothing). The gate still decides: a
combination whose matrix row is not verified is refused (exit 1), and repeat text that does not parse exits 2.
Dates with and without a time, start dates, and 2Do's automatic alert are handled as described in [tasks.md](tasks.md).

Verified behaviour that is easy to get wrong, each checked on the iPhone:

- a monthly task completed months late moves one month per completion (no skipping);
- completing a task with a start date moves the start by as many days as the due date;
- completing a recurring project with some items done and some open;
- changing a checklist's date removes its items' copies of the old date;
- undoing a completion before and after it was synced;
- completing every repeat kind Twydo creates gives the phone's result: a monthly task on the
  31st moves to February 28, then March 28 (no memory of the 31st); 29 February moves to
  28 February; the last occurrence of an ended series is completed in place; "from completion"
  counts from the completion day even when the start date is later.

Not verified yet, so refused for writing in verified mode: setting, removing or changing a
"from completion" repeat, and any repeat change on a project or checklist; converting an empty project to a checklist; editing an existing "from completion"
task; completing a repeat with an end date; completing a recurring project that ends or
repeats from completion; changing the interval of an Nth-weekday repeat (not supported by 2Do);
reopening a repeating task itself (the last occurrence of an ended series: use undo);
repeating checklists; a repeating
item inside a repeating project (creating one included); nested projects and checklists; moving,
deleting or reopening a single item on its own; editing alerts. Converting a project that has
items is not supported by 2Do at all.

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
