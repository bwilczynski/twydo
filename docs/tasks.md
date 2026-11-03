# How tasks behave

Twydo follows 2Do's rules, because both apps edit the same tasks. Everything here was checked
against 2Do on an iPhone; where Twydo cannot write something yet, it says so (see
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
  items yet. Converting a repeating task to a checklist is refused (repeating checklists are not
  verified).
- **Checklist with items to project**: the items stay, with their dates as they were.
- **Empty project or checklist to task**: allowed.
- **A project with items cannot be converted**, neither to a checklist nor to a task. The iPhone
  greys the selector out for it, so Twydo refuses with `… is not supported by 2Do`. Move or
  finish the items first.
- Converting to the type it already has does nothing.

To fill a parent, `a` adds an item (on a plain task it asks first whether to make it a project
or a checklist). A checklist's items have no dates, repeat or alerts of their own.

## Dates

- **Due date**: date-only (`Fri`) or with a time (`Fri 10:00`). A date-only task becomes overdue
  the day after its due date; a timed one, once its time has passed.
- **Start date**: the day you plan to start. A task whose start date is still in the future is
  out of the way: it is not in Today, All or its list, but in **Scheduled**, until that day
  comes. Start dates have no time.
- A **checklist item** has no dates of its own; date keys and the form's Due and Start are refused
  on an item. Some items still carry an old copy of the checklist's date (2Do gives items a copy
  when the checklist already had a date when they were added). When the checklist gets a new
  date, Twydo removes those copies in the same write, as 2Do does.

## Completing, reopening, moving, deleting

On a **task**: `x` (or `Space`) completes it; on a completed task it reopens it. `m` moves it to
another list, `dd` deletes it, `u` undoes the last write and `Ctrl-r` redoes it. A moved task gets
a new UID in its new list, as in 2Do.

On a **checklist or project**, every write acts on the whole family, as on the iPhone:

- **Completing** a parent with open items first asks (`complete "<title>" and its <n> open
  items? x to confirm`; `tdo op complete` needs `--all`). Then every open item is completed with
  the parent's completion time. A repeating project item is completed with its repeat kept, so in
  practice it stops repeating until the project is reopened. A parent whose items are all done
  completes at once.
- **Completing the last item** does not complete the parent.
- **Reopening** a parent reopens every completed item, whatever its state before.
- **Moving** a parent moves every item with it; all get new UIDs and the items' links follow.
- **Deleting** a parent deletes its items. History copies of a repeating task stay.
- Cancelled items (from other apps) are left as they are by complete and reopen, and go with the
  family on move and delete.
- Moving or deleting **one item on its own**, and reopening an item of a completed parent, are
  refused (not verified yet).

## Repeats

A repeating task shows ↺ and its rule (`weekly`, `monthly`, …); the Details pane spells it out
(`Every Mon`). 2Do can express many rules; Twydo reads all of them and can write all of them, but in the default verified mode
it **changes only monthly tasks on a day from 1 to 28 and yearly tasks** until the others are
checked against the iPhone. Other repeats (daily, weekly, on set weekdays, the 29th–31st, the
last day of the month, "the second Tuesday", February 29, repeat from completion, repeats with an
end) are shown, and a write to them, including setting one, is refused with the reason, for
example `weekdays-set repeats are not verified yet`. `tdo check` lists the repeat kinds in your data and whether each is verified.

**Completing a repeating task** does two things, as 2Do does:

1. a **completed copy** of this occurrence is added (it shows in Done);
2. the task itself, with the same identity, **moves to the next occurrence**.

In the TUI the cursor stays on the row where the task was, as after completing any other task;
it does not follow the task to its next date.

How the next occurrence is found:

- **One interval per completion.** Missed occurrences are not skipped: a monthly task due
  August 15 and completed in November moves to September 15, still overdue. Complete it once per
  missed month (each completion adds a copy), or change its date.
- **A start date moves by the same number of days as the due date.** A yearly task due
  2027-03-01 with a start on 2027-02-13 moves to due 2028-03-01 (366 days later, a leap year)
  and start 2028-02-14.
- **A due time is kept**, and a relative alert (such as 2Do's automatic one at the due time)
  moves with the due date. The completed copy keeps its alert.

**Undo** after completing a repeating task removes the completed copy and moves the task back,
also after the change has been synced to the phone.

**Recurring projects** (a repeating project): completing one asks first if items are open, then
moves the project to its next occurrence, adds a completed copy of the project with a completed
copy of every item under it (items that were already done keep their own completion time), and
reopens all the live items. The **items' dates do not move** with the project, as in 2Do.

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
  them (it offers "last Monday" and so on instead), so they stay unverified.

## Views and date groups

| View | Shows |
|---|---|
| **Today** | open tasks that are overdue, due today, or started (start date reached) |
| **All** | every open task that has started, from lists not excluded from All |
| **Scheduled** | open tasks whose start date is in the future, sorted by start date under one Scheduled header |
| **Done** | completed tasks, newest first, grouped by completion day |
| **Next 3 Days** | open tasks due today, tomorrow or the day after (not overdue) |
| **a list** | its open tasks that have started |

The number beside a view is `due│total`: tasks due today or overdue, then all of them (checklist
items are not counted; Done shows its total only).

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
| Future | due later |
| No Due Date | no due date |

Within a group, tasks sort by due date (on the same day, timed tasks before date-only ones), then
by title. A list can sort by `due`, `start`, `creation` or `title`, ascending or descending
(`[lists.<id>] sort` in [config.md](config.md)); sorting by title or creation shows one list without groups.

## Alerts

Twydo keeps every alert a task has, shows them in the Details pane (`15 min before due`), and
copies them with a repeating task's completed occurrence. Setting or removing a due time adds or
drops 2Do's automatic at-due-time alert, as 2Do does. Editing alerts in the TUI (`a`) and
desktop notifications are not available yet.
