# How tasks behave

Twydo follows 2Do's task-data rules, because both apps edit the same tasks. Writable behavior
is checked against 2Do on an iPhone; view presentation includes Twydo's own choices described
below. Where Twydo cannot write something yet, it says so (see
[compatibility.md](compatibility.md) for why). For how this is stored in the `.ics` files, see
[data-model.md](data-model.md).

## Kinds of task

| Kind | What it is |
|---|---|
| **Task** | A single to-do, with an optional due date (with or without a time), start date, repeat, alerts and notes |
| **Checklist** | A task with items. Items are simple: they have no dates of their own and take the checklist's |
| **Project** | A task with items that are full tasks: each has its own due date, start date and repeat |

In the TUI, a checklist shows `[3]` (its number of items) and the word `checklist`; a project shows
`project`. `Enter` expands or collapses a parent in All and in lists; the Details pane lists its
items. Today, Scheduled and Next 3 Days show a checklist's items under it.

Twydo reads nested projects and checklists (a parent inside a parent) but does not write to them.

### Changing a task's type

`ct`, `cp` and `cc` (or the card's Type field) convert the selected task, as the iPhone's type
selector does:

- **Task to checklist or project**: the task keeps its title, dates, repeat and notes, and has no
  items yet. A repeating task can become a checklist only for the verified plain monthly,
  interval-1, date-only day 1–28 due-anchored shape without end/start/alerts/delay.
- **Checklist with items to project**: the items stay, with their dates as they were.
- **Empty project or checklist to task**: allowed.
- **A project with items cannot be converted**, neither to a checklist nor to a task. The iPhone
  greys the selector out for it, so Twydo refuses with `… is not supported by 2Do`. Move or
  finish the items first.
- Converting to the type it already has does nothing.

In verified mode task to checklist, task to project, checklist to project and empty checklist or
project to task are allowed. Empty project to checklist is still refused
(`… is not verified yet (iphone_baseline)`), as is any conversion not listed
([compatibility.md](compatibility.md#verified-mode)).

To fill a parent, `a` adds an item (on a plain task it asks first whether to make it a project
or a checklist). A checklist's items have no dates, repeat or alerts of their own.

## Dates

- **Due date**: date-only (`Fri`) or with a time (`Fri 10:00`). A date-only task becomes overdue
  the day after its due date; a timed one, once its time has passed.
- **Start date**: the day you plan to start. A task whose start date is still in the future is
  shown in **Scheduled** and the **Future** section of All and its list until that day
  comes. It stays out of Today and Next 3 Days until it starts. Start dates have no time.
- A **checklist item** has no dates of its own; date keys and the form's Due and Start are refused
  on an item. Some items still carry an old copy of the checklist's date (2Do gives items a copy
  when the checklist already had a date when they were added). When the checklist gets a new
  date, Twydo removes those copies in the same write, as 2Do does.

## Completing, reopening, moving, deleting

On a **task**: `x` (or `Space`) completes it; on a completed task it reopens it. `m` moves it to
another list, `dd` deletes it, `u` undoes the last write and `Ctrl-r` redoes it. A moved task gets
a new UID in its new list, as in 2Do.

On a **checklist or project**, parent completion, reopening, moving and deletion act on
the family, where those operations are verified:

- **Completing** a parent with open items first asks (`complete "<title>" and its <n> open
  items? x to confirm`; `tdo op complete` needs `--all`). Then every open item is completed with
  the parent's completion time. A repeating project item is completed with its repeat kept, so in
  practice it stops repeating until the project is reopened. A parent whose items are all done
  completes at once.
- **Completing the last item** does not complete the parent.
- **Reopening** a parent reopens every completed item, whatever its state before.
- **Moving** a parent moves every item with it; all get new UIDs and the items' links follow.
- **Deleting** a parent deletes its items, including history linked as its children.
  History outside that family stays. This covers
  a recognized top-level checklist or project with flat items, empty or not, open, completed or
  cancelled, including recurring parents and independently recurring items. Only the selected
  family is removed; unrelated resources and history outside it stay unchanged, and one `u` restores
  it. Broken or ambiguous links, nested families, a plain task with subtasks, unknown types and
  unrecognized rules stay refused.
- Cancelled items (from other apps) are left as they are by complete and reopen, and go with the
  family on move and delete.
- Moving one item and reopening an item of a completed parent remain refused.
- `dd` deletes the **selected item of a checklist or project**, including a completed or
  last/only item and an item under a completed parent. The empty parent keeps its type and
  state. Only the selected resource is removed; parent, siblings, dates, recurrence, alerts
  and history stay unchanged. Recognized recurring families and independently recurring
  project items use the same operation, without requiring recurrence-execution permission.
  Parent/sibling or membership changes make apply, undo, redo and recovery refuse safely.
  Broken links, duplicate identities, unreadable metadata, unrecognized rules and nested
  families remain refused. The [representative phone session](reference/phase0/issue27-item-deletion.md)
  and local preservation tests establish this operation invariant.

Creating, editing schedules and completing a **recurring checklist** have narrower verified support: plain monthly day 1–28, interval 1,
date-only, due anchored, no end/start/alerts/delay. Completing it copies the parent/items to
history, preserves prior item completion times, then reopens live items and removes inherited
date copies. It still needs its own final tick. Cancelled items and independent item starts or
times are refused. In this scope you can create the checklist directly, convert an existing
monthly task, add items, edit parent/item titles and notes, and tick items. Individual item
deletion follows the general rule above. Date/start/time/interval and other type edits remain refused;
pure monthly/weekly repeat changes are covered separately below.

## Repeats

A repeating task shows ↺ and its rule (`weekly`, `monthly`, …); the Details pane spells it out
(`Every Mon`). 2Do can express many rules and Twydo reads all of them. In the default verified
mode, **creating** a task with any repeat but the three below is allowed (daily, weekly, on set
weekdays, monthly on a day or the Nth weekday, yearly including February 29, repeat from
completion, with an end by count or date), and so is creating a checklist, a project, a
recurring project and their items. **Editing** an
existing repeating task: daily, weekly, monthly and yearly tasks can be edited (the due date,
title, other fields, and the interval: every 2 weeks instead of every week). The same interval
change is accepted for monthly on the 29th to 31st, for February 29 and for repeats with an
end, which share the same interval encoding. The interval of an Nth-weekday repeat (`2nd tue`,
`last fri`) does not change: 2Do has none for it and keeps the ordinal where the interval would
go (`changing the interval of an Nth-weekday repeat is not supported`). A repeat on set
weekdays (`mon thu`, `workdays`) repeats every week only and takes no interval, in any write
mode: Twydo could not complete it (`an interval is not supported on a repeat on set
weekdays`). Existing standalone monthly **from-completion** tasks now support title,
notes, due date/time and start edits at interval 1, without an end. Source and resulting
due dates must be on days 1–28; date-only and floating times with the captured automatic
due alarm are supported, while custom alarms and zoned dates remain refused. A separate
pure, date-only interval change from monthly interval 1 to 2 is verified through both CLI
and card. Ordinary weekly or interval-2 from-completion field edits remain unverified.

**Setting or removing a repeat:** plain daily, weekly, monthly and yearly rules can be set,
removed or changed between those classes. Newly verified daily/yearly transitions are due
anchored and have no end or weekday selector; they do not enable end/anchor/Nth-rule changes.
The prior monthly/weekly support, including a monthly count end, remains. Existing checklists
can set, change or remove plain monthly/weekly rules with interval 1, date-only due, no end,
start, alerts or completion anchor; monthly rules require a due day of 28 or less. Monthly
checklist title/notes and task-to-checklist conversion are covered above. Other ordinary
schedule/interval/type edits, other checklist rules and project repeat changes remain refused. Standalone tasks can
make the captured completion-anchored changes: plain or monthly due to monthly completion,
monthly completion to weekly completion or no repeat, and weekly completion to weekly due.
These require interval 1, day 1–28 date-only due, no end/start/alert/delay. These are pure repeat changes;
mixing ordinary edits or a due change into the same repeat save stays refused. Daily/yearly,
weekday-selector and family from-completion transitions remain unverified. A plain task
still linked to another series (including reopened history) cannot acquire completion
recurrence; its identity transition is unverified.

**Three kinds Twydo never writes:** the last day of the month, the last weekday and the first (or
Nth) weekend day. 2Do 3.91 on the iPhone, the last version available for it, misreads them
(the last day shows as plain monthly, the last weekday as "every workday", the weekend day as
"every weekend day") and its picker cannot create them. Twydo shows them when it finds them, but
refuses to write them in every mode, for example `months-last-day repeats are not supported by
2Do on the iPhone`.

**Completing** supports the previously verified kinds: daily, weekly, set weekdays,
monthly on any day, the Nth or last weekday, yearly including 29 February, from completion, and
with an end by count. Each was checked against the iPhone: the phone completed one copy of a
task, Twydo an identical copy, and the two results were the same, byte for byte. An **end date** (`until`) is supported for top-level plain daily/monthly day 1–28 tasks with
interval 1, date-only due/end, a due-date anchor, and no children/start/alerts/delay. Other until
shapes remain refused. The [compatibility page](compatibility.md#what-is-verified-today) lists
the exact newly verified operations and remaining family restrictions.
`tdo check` lists the repeat kinds in your data and whether each is verified.

**Completing a repeating task** does two things, as 2Do does:

1. a **completed copy** of this occurrence is added (it shows in Done);
2. the task itself, with the same identity, **moves to the next occurrence**.

In the TUI the cursor stays on the row where the task was, as after completing any other task;
it does not follow the task to its next date.

How the next occurrence is found:

- **From the current due date alone**, by the rule: every N days or weeks; the next day of a
  set of weekdays (a due date off the set moves to the set's next day: Mon and Thu due on a
  Wednesday moves to Thursday); every N months or years by the calendar, a day the shorter month
  lacks becoming its last day. Twydo, like 2Do, keeps no memory of the original day: a monthly
  task on January 31 moves to February 28, then to March 28, not March 31; a yearly one on
  29 February moves to 28 February the next year, and stays on the 28th. The Nth or last weekday
  moves to the next month that has one (last Friday: December 25 to January 29).
- **A repeat from completion** counts from the day you complete it, whatever its due or start
  date: a monthly task due December 7 completed on November 4 moves to December 4, and a start
  date of December 20 moves by the same 3 days back, to December 17.
- **An end by count** counts down with each completion (`COUNT=3` becomes `COUNT=2`).
  The last count occurrence, or a supported until occurrence whose next date would be after
  the inclusive end, completes in place: no new history copy, and rule/due/end stay in Done.
  Undo restores it. The captured top-level date-only interval-1 daily/plain-monthly until
  originals can also be reopened in place; count-ended, family-ended and other shapes still
  say `reopening the last occurrence of an ended repeat is not supported; use undo`.

- **One interval per completion.** Missed occurrences are not skipped: a monthly task due
  August 15 and completed in November moves to September 15, still overdue. Complete it once per
  missed month (each completion adds a copy), or change its date.
- **A start date moves by the same number of days as the due date.** A yearly task due
  2027-03-01 with a start on 2027-02-13 moves to due 2028-03-01 (366 days later, a leap year)
  and start 2028-02-14.
- **A due time is kept**, and a relative alert (such as 2Do's automatic one at the due time)
  moves with the due date. The completed copy keeps its alert.

**Undo** after completing a repeating task removes the completed copy and moves the task back
(or, for the last occurrence completed in place, makes it open again), also after the change has
been synced to the phone.

**Recurring projects** (a repeating project): completing one asks first if items are open, then
moves the project to its next occurrence, adds a completed copy of the project with a completed
copy of every item under it (items that were already done keep their own completion time), and
reopens all the live items. The **items' dates do not move** with the project, as in 2Do.
Completing a recurring project whose repeat has an end (count or date) or counts from
completion is refused in every mode (`completing a recurring project that ends or repeats from
completion is not verified yet`): 2Do's behaviour was captured for plain repeats only.

The edit card (`r` on any task, or `i` then the Repeat field) sets, changes or removes a repeat:
you type the rule, such as `monthly`, `2nd tue` or `mon thu after done`
([tui.md](tui.md#the-edit-card)). `tdo op add` and `tdo op edit` take the same text with
`--repeat`.

| Typed | Repeats |
|---|---|
| `none` or empty | never (removes a repeat) |
| `daily`, `3d`, `every 3 days` | every N days (2Do's "Every Day" is the same rule) |
| `weekly`, `biweekly`, `2w`, `every 2 weeks` | every N weeks |
| `monthly`, `3m`, `every 3 months` | every N months, on the due date's day |
| `yearly`, `2y`, `every 2 years` | every N years, on the due date |
| `mon thu`, `every mon and thu` | those weekdays |
| `workdays` | the `work_days` of the config, default Monday to Friday ([config.md](config.md)) |
| `last day`, `last day every 2m` | the last day of the month |
| `2nd tue`, `last fri` | that weekday of each month |
| `1st weekday`, `last weekend day` | that workday (Monday to Friday) or weekend day of each month |

Modifiers, after any rule: `after done` (count from the completion date), `until 2027-06-01`
(or any due-style date) and `10 times` (stop after that many).

- A repeat needs a due date. Set one on a task without it and the task becomes due today, as on
  the iPhone.
- The field never names a day of the month: monthly and yearly repeats follow the due date, so
  set the day there.
- `last day`, `last weekday` and `1st weekend day` can be typed, but the iPhone cannot create
  them (it offers "last Monday" and so on instead), and Twydo refuses to write them (see
  [Repeats](#repeats)).

## Views and date groups

| View | Shows |
|---|---|
| **Today** | open tasks that are overdue, due today, or started (start date reached) |
| **All** | open tasks (including future-start tasks) and tasks completed in the last 30 days, from eligible lists not excluded from All |
| **Scheduled** | open tasks whose start date is in the future, sorted by start date under one Scheduled header |
| **Done** | completed tasks, newest first, grouped by completion day |
| **Next 3 Days** | open tasks due today, tomorrow or the day after (not overdue) |
| **a list** | its open tasks (including future-start tasks) and tasks completed in the last 30 days |

All and ordinary lists always show future-start work under **Future** and completed work
under **Done** at the bottom for the last 30 calendar days including today, newest completions
first. Older completions and items without a completion date remain in the dedicated Done
view; nothing is deleted. Empty sections disappear. These
sections also appear with title/creation sorting; there are no visibility toggles or settings.
Completed tasks are excluded from Today, Scheduled and Next 3 Days. A list's `show_in_done`
setting affects only the dedicated Done view, not its completed rows in All or its own list.

The number beside a view is `due│total`: unfinished tasks due today or overdue, then all
eligible unfinished tasks, including future-start work. Completed rows do not inflate these
numbers; checklist items are not counted, project children are, and Done shows its completed
total only.

Tasks are grouped by due date, in this order:

| Group | Contains |
|---|---|
| Overdue | due before today, or a time already past |
| In Progress | Today only: started, not yet due |
| Later Today | Today only: due later today at a time |
| Today, Tomorrow, Day After Tomorrow | due on that day |
| Next 7 Days | due in 3–6 days |
| Next 2 Weeks | due in 7–13 days |
| Next 30 Days | due in 14–29 days |
| Future | due later, or not yet started (All and ordinary lists) |
| No Due Date | no due date |
| Done | completed tasks (All and ordinary lists), regardless of due/start dates |

Within a group, tasks sort by due date (on the same day, timed tasks before date-only ones), then
by title. A list can sort by `due`, `start`, `creation` or `title`, ascending or descending
(`[lists.<id>] sort` in [config.md](config.md)); sorting by title or creation leaves active rows
without date groups, alongside any Future and Done sections.

Items nested under a checklist or project keep their own sequence: `DisplayOrder` ascending,
then creation time for tied positions. Missing or invalid positions count as zero; missing
creation times come last, and remaining ties use UID for a stable order. The list's sort does
not reorder nested items. In All and ordinary lists, children in the same active/future-start/
completed section as their parent appear beneath it when expanded. Children in a different
section appear separately with parent context, so a collapsed open parent cannot hide its
completed children. Ordinary due-date buckets do not split an active family.
Project items shown independently in Today and other flat date
views still use that view's ordinary date sort.
The Details pane uses the same item sequence. For legacy items tied on both position and
creation time, UID is only a deterministic fallback: a family move/history copy can change
that tie order because it assigns fresh UIDs. Unknown historical insertion order cannot be
recovered from tied fields; Twydo does not renumber existing items to guess it.
An explicit `J`/`K` move or anchored `o`/`O` insertion deliberately gives the resulting
complete sibling sequence zero-based `DisplayOrder` ranks, including completed items.
Only differing ranks are rewritten, with normal modification timestamps; creation times
and unrelated metadata remain intact. `a` and subsequent Enter-chain items append.
Ordering is independent of recurrence, dates, alarms and completion state: those fields
are preserved, so recurring checklists/projects use the same `DisplayOrder` operation.
Phone display requires **Manually** selected in 2Do. Anchored insertion additionally needs
the existing new-item creation checks. Sorting preferences and independently displayed
project-item scheduling are unaffected.

## Alerts

Twydo keeps every alert a task has, shows them in the Details pane (`15 min before due`), and
copies them with a repeating task's completed occurrence. Setting or removing a due time adds or
drops 2Do's automatic at-due-time alert, as 2Do does. Editing alerts in the TUI (`a`) and
desktop notifications are not available yet.

### Detached history reopening

Reopening a completed history task makes that occurrence open without recreating or
advancing its repeat. It keeps its UID and foreign RUID, dates, starts, alarms and
unrelated metadata; the original live series stays byte-identical. Only status,
completion fields and modification stamps change. Readable JSON history with the
captured inactive recurrence fields and no RRULE is required.

Reopening a flat history project or checklist reopens the parent and **every
completed child**, including children completed before the parent. Open and
cancelled children stay unchanged. This is one journal/undo step; queue, undo,
redo and crash recovery retain the family dependencies and refuse stale members.
The project behavior is directly observed in Plan 6 H7; checklist and unchanged
scheduling combinations are supported by exact preservation tests.

An individual detached history item remains refused, including under an open
history parent: the live-child H3 captures do not prove that separate operation.
Reopening history does not undo the original recurrence advancement. Use undo for
that; ended originals retaining RRULE use their separately bounded capability.
