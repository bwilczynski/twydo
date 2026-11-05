# Data model

What Twydo's data looks like on disk, and how it maps to what 2Do writes over CalDAV. Twydo has
no database of its own: the files below are the data. For how tasks behave when you complete,
move or repeat them, see [tasks.md](tasks.md).

## The vdir

A vdir is the folder [vdirsyncer](https://vdirsyncer.pimutils.org/en/stable/vdir.html) keeps in
step with the CalDAV server:

```
<vdir>/
  <list>/                  one folder per CalDAV calendar collection = one 2Do list
    displayname            the list's name
    color                  the list's colour, e.g. #3B82F6 (2Do adds an alpha byte: #379417FF)
    <uid>.ics              one file per task, holding one VCALENDAR with one VTODO
```

The folder names are vdirsyncer's collection ids, not list names. `displayname` and `color` are
synced with the server's properties when vdirsyncer's metasync is on ([sync.md](sync.md)). New
files Twydo writes are named `<uid>.ics`; existing files keep whatever name they have.

## A task as 2Do writes it

This is a project item created on the iPhone, from the synthetic test fixtures
(`fixtures/phase0/v1/plan6-list1/01-create`):

```
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//2DOAPP.COM//2Do 3.8//EN
CALSCALE:GREGORIAN
BEGIN:VTODO
DTSTAMP:20261102T152814Z
LAST-MODIFIED:20261102T152814Z
UID:e668818024cc48f4a404fb9ced368789
RELATED-TO:09c49c2cd76f483498fc2ff3a0f66160
SEQUENCE:0
SUMMARY:p1
DESCRIPTION:
STATUS:NEEDS-ACTION
CREATED:20261102T152712Z
DTSTART;VALUE=DATE:20261205
DUE;VALUE=DATE:20261205
X-2DOAPP-METADATA;SHARE-SCOPE=GLOBAL:<2Do 
 Meta>%7B%22RecurrenceValue%22%3A0%2C%22RecurrenceEndRepetitionsOrig%22%3A0
 %2C%22RecurrenceEndRepetitions%22%3A0%2C%22TaskDuration%22%3A0%2C%22TaskTy
 pe%22%3A0%2C%22RecurrenceType%22%3A0%2C%22StartDayDelay%22%3A0%2C%22isExpa
 ndedToShowChildProjects%22%3A0%2C%22RecurrenceFrom%22%3A0%2C%22RecurrenceE
 ndType%22%3A0%2C%22IsStarred%22%3A0%2C%22parentuid%22%3A%2209c49c2cd76f483
 498fc2ff3a0f66160%22%2C%22RUID%22%3A%22%22%2C%22DisplayOrder%22%3A0%7D</2D
 o Meta>\n
X-2DOAPP-SYNC-IMAGE:1
X-2DOAPP-SYNC-AUDIO:1
X-2DOAPP-AUDIO-DURATION:0.000000
END:VTODO
END:VCALENDAR
```

## Standard properties

| Property | Meaning |
|---|---|
| `UID` | The task's identity. 2Do uses 32 lowercase hex digits; Twydo creates UIDs the same way. |
| `SUMMARY` | Title |
| `DESCRIPTION` | Notes (2Do writes an empty one on every task) |
| `DUE` | Due date: `VALUE=DATE` for a date-only task, a date-time for a timed one |
| `DTSTART` | **Not the start date.** 2Do writes a copy of `DUE` on every dated task and omits it on undated ones. The start date lives in the metadata (below) |
| `STATUS`, `COMPLETED`, `PERCENT-COMPLETE` | Open: `NEEDS-ACTION`. Completed: `COMPLETED`, plus the completion time and `100`. `CANCELLED` tasks are read and left alone |
| `RRULE` | The repeat rule (see [Repeats](#repeats)) |
| `RELATED-TO` | On a checklist item or project item: the parent's `UID` |
| `VALARM` | Alerts. 2Do adds an automatic at-due-time alert (marked `X-2DOAPP-ISAUTO:1`) to a timed due date and drops it when the time is removed; Twydo does the same |
| `CREATED`, `DTSTAMP`, `LAST-MODIFIED`, `SEQUENCE` | Stamps. Every write updates `DTSTAMP` and `LAST-MODIFIED` |

## 2Do's metadata: `X-2DOAPP-METADATA`

Over CalDAV, 2Do stores what iCalendar cannot say in `X-2DOAPP-METADATA`: the text
`<2Do Meta>`, a percent-encoded JSON object, `</2Do Meta>`, and an escaped newline. Decoded, the
example above is:

```json
{"RecurrenceValue":0,"RecurrenceEndRepetitionsOrig":0,"RecurrenceEndRepetitions":0,
 "TaskDuration":0,"TaskType":0,"RecurrenceType":0,"StartDayDelay":0,
 "isExpandedToShowChildProjects":0,"RecurrenceFrom":0,"RecurrenceEndType":0,"IsStarred":0,
 "parentuid":"09c49c2cd76f483498fc2ff3a0f66160","RUID":"","DisplayOrder":0}
```

The keys Twydo reads or writes:

| Key | Meaning |
|---|---|
| `TaskType` | 0 task, 1 checklist, 2 project |
| `StartDate` | The start date, as epoch seconds at UTC midnight of the start day. Absent: no start date |
| `parentuid` | On an item: the parent's JSON `uid` when present; otherwise its CalDAV UID. `RELATED-TO` always uses the CalDAV UID |
| `uid` | On a checklist or project: its parent identity, which may differ from its CalDAV UID after a phone conversion |
| `RUID` | The repeat series a task belongs to: on a live repeating task its own UID (or empty), on a completed history copy the UID of the task it was copied from |
| `RecurrenceType`, `RecurrenceValue`, `RecurrenceFrom` | 2Do's own copy of the repeat rule: type, interval, and 2 = repeat from the completion date |
| `RecurrenceEndType`, `RecurrenceEndRepetitions` | The repeat's end |
| `duetime` | The due time as HHMM (`1430`) on a timed due date; absent without a time |
| `IsStarred` | 0 or 1 |
| `DisplayOrder` | Nested checklist/project item position, ascending. Creation time breaks ties (older phone captures often contain zero on every item) |

New checklist/project items receive an append position: the larger of the existing sibling
count (including completed items) and one more than the highest valid sibling position.
An empty parent starts at zero. Twydo preserves existing sibling resources and creation
timestamps; positions beyond the supported integer range are refused instead of wrapping.
The ordering writer's phone return check is tracked in the
[issue 35 capture notes](reference/phase0/issue35-checklist-order.md).

Other keys (`TaskDuration`, `StartDayDelay`, …) are kept as they are. When Twydo
changes a field it changes every representation of it: moving a start date updates
`StartDate`, completing a repeat updates `DUE`, its `DTSTART` copy and `StartDate` together.
Twydo edits the JSON field by field and keeps the key order and the text around the markers, so
the rest of the value stays byte for byte the same.

2Do also has an older encoding, a line of `@key:value` tokens (`@tasktype:checklist`,
`@repeatfromcompletion`, …). Twydo reads it too, and the sample vdir in the tests uses it, but
2Do writes the JSON form over CalDAV. Metadata that cannot be decoded is reported and makes the
task read-only.

The other `X-2DOAPP-*` properties (`X-2DOAPP-SYNC-IMAGE`, `X-2DOAPP-SYNC-AUDIO`,
`X-2DOAPP-AUDIO-DURATION`) are kept on tasks that have them and left out of tasks Twydo
creates, because their meaning is not known.

## Projects and checklists

A checklist or project is a task with `TaskType` 1 or 2. Its items are separate tasks in the same
list, each pointing at it with `RELATED-TO` and `parentuid`. Twydo reads nested projects and
checklists but refuses writes to them (not verified yet).

- A **checklist** item takes its dates from the checklist; items have no dates of their own
  that 2Do shows. Some items still carry an old copy of a date; when the checklist gets a new
  date, Twydo removes those copies in the same write, as 2Do does.
- A **project** item is a full task: its own due date, start date and repeat.

## Repeats

A repeating task has an `RRULE` and the same rule in the JSON `Recurrence*` keys. Twydo reads every
rule shape 2Do writes and sorts it into a **rule class**, the key the compatibility matrix uses:

| Class | Example |
|---|---|
| `days`, `weeks` | every 3 days; every 2 weeks |
| `weekdays-set` | every Monday and Thursday |
| `months-day-1-28`, `months-day-29-31`, `months-last-day` | the 15th; the 31st; the last day |
| `months-nth-weekday`, `months-nth-workday`, `months-nth-weekend-day` | the second Tuesday; the first workday |
| `years`, `years-feb-29` | every year; on February 29 |

A class can carry `+from-completion` (repeat from the completion date) and `+count` or `+until`
(an end). `tdo check` lists the classes in your data and whether each is verified.

**Completing a repeating task** writes two files (copy-and-advance, as 2Do does):

- a **history copy**: the task as it was, with a new UID, completed, without `RRULE`, its JSON
  repeat fields set to 0 and `RUID` pointing at the series;
- the **live task**, same UID, with `DUE` (and its `DTSTART` copy and `StartDate`) moved to the
  next occurrence, and a `COUNT` (with JSON `RecurrenceEndRepetitions`) one lower.

The copy's JSON end fields are 0 too. When the series ends (`COUNT=1`, or the next date is after
`UNTIL`), there is no copy: the task itself is completed, its `RRULE` kept.

A repeating checklist's history parent receives a new JSON `uid` even when its live
parent has a distinct phone-internal identity. The live identity remains unchanged,
and the copied items link to the new history parent. This is captured in the
[B-only checklist session](reference/phase0/plan8-checklist-29-checklist.md).

Completing a repeating project also copies each of its items under the project's history copy.

## Moving a task

A task moved to another list gets a **new UID** in the destination list, and the old file is
removed, as 2Do does. Self-references in the metadata (`uid`, and `RUID` on a live task) follow the
new UID; items moved with their parent point at the parent's new UID.

## What Twydo preserves

- A file Twydo does not change is never rewritten.
- In a file it changes, only the properties of the operation change, plus `DTSTAMP`,
  `LAST-MODIFIED` and their metadata counterparts. Every other line keeps its original bytes,
  including line folding, line endings, property order, unknown properties and parameters, and
  unknown components.
- A file that cannot be parsed is listed under `:errors` in the TUI and left untouched.

## Twydo's own files

Outside the vdir, under the state directory ([architecture.md](architecture.md)): the operation
journal, sync status and manifest, and conflicts (both versions of a conflicted task). All are
plain JSON or `.ics` files you can inspect. None of them is a second copy of your tasks.
