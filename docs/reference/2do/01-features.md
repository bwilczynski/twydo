# 01 — Features and behaviour

Legend: **[obs]** observed · **[inf]** inferred · **[?]** unresolved.

---

## 1. Task types

| Type | What it is | Children | Row icon |
|---|---|---|---|
| **Task** | A single to-do | none | round checkbox ○ (repeating tasks show a circular-arrow checkbox ↺) |
| **Project** | A task that contains sub-tasks; each sub-task has its own dates, tags, priority, alerts | full tasks | briefcase/box icon with a count badge (e.g. "4") and a disclosure chevron |
| **Checklist** | A task with simple checklist items | items with **no dates or alarms** | calendar-like box with count badge, chevron |

- Create: New Task ⌘N, New Sub-Task ⌃⌥N, New Project ⌥⌘N, New Checklist ⌃⌘N; or "Project/Checklist from Selection". [obs]
- Convert between types: ⇧⌥⌘1 Task, ⇧⌥⌘2 Project, ⇧⌥⌘3 Checklist. [obs]
- Converting a project → checklist warns: "All its sub-tasks will lose their dates and alarms". [obs]
- Converting a project/checklist with children → task asks for confirmation. [obs]
- Completing a project/checklist with unfinished children asks "still contains unfinished tasks. Are you sure?" [obs]
- In the quick entry window a **Task / Project / Checklist** selector sits bottom-left. [obs]
- Hitting ↩ while editing a sub-task title creates the next sub-task. [obs]
- Sub-tasks appear indented under their parent; in other views (Today, All) a sub-task shows a second line `↳ [briefcase] Parent name`. [obs]
- **Project View** (⌥⌘J): drill into one project and see only its sub-tasks. [obs]
- Projects and checklists collapse/expand (`c` collapses; ⌥⌘↑/↓ collapse/expand all). [obs]

## 2. Task fields (in scope)

| Field | Notes |
|---|---|
| Title | required |
| List | every task belongs to exactly one list (`calendaruid`) |
| Parent | project/checklist it belongs to (`parent`) |
| Notes | free text, **Markdown** supported (M toggle in editor); notes can link to other tasks ("Linked from" backlinks) [obs] |
| Due date | date only, or date + due time |
| Due time | optional; stored separately (`duetime`, default 999999 = "no time") [obs] |
| Start date (+ optional time) | hides the task until then (see §4) |
| Duration | minutes; shown as "30 m", "1 h", "3 h" [obs] |
| Priority | None / Low / Medium / High |
| Starred | boolean flag, independent of priority |
| Tags | many per task |
| Repeat | see §6 |
| Alerts | one or many; see §7 |
| Action | optional "launch" action: call, message, email, visit/drive, open URL, search (⌥1–⌥6 in the action picker) [obs] — optional for the port |
| Created / modified timestamps | "Created: Today at 6:57 PM" shown above the editor [obs] |

## 3. Due dates, times, overdue

- A task is **overdue** when its due date is before today; with the setting "Due Time taken into account when marking a task Overdue" on, a task due today at 10:00 is overdue at 10:01. [obs]
- **Default due time** setting (e.g. 7:30 AM): tasks without a due time get it when their due date is modified. [obs]
- Optional **default due date for new tasks** ("relative to the current date"); does not apply to tasks created in Today. [obs]
- **New tasks created while viewing Today are due today.** [obs]
- Display uses relative text: "4 days ago", "Yesterday", "Today, 7:00 AM", "Tomorrow, 12:15 PM", "Thursday 11:30 AM", "Due in 13h 31m", "Due in %@ days", "Overdue by %@ hours", "Starts in 4 days". [obs]
- Overdue tasks: **title and date drawn in dark red**. [obs]
- Quick due shortcuts on the selected task(s): `t` Due Today, `y` Due Tomorrow, `w` Due This Weekend, `e` Due Time…, `]` +1 day, `⇧]` +1 week, `[` −1 day, `⇧[` −1 week, `⌥[` remove due date, `⇧⌘[` remove due time. [obs]

## 4. Start date, "Scheduled", "In Progress"

- "**A start date keeps the task hidden from your lists and Today until it's time to begin.**" [obs]
- Tasks with a future start date are **Scheduled**. They're hidden by default. Show them with ⌘K (Show Scheduled), or click the "Show Scheduled / Hide Scheduled" link at the bottom of a list. Scheduled rows show a clock icon instead of the checkbox and a muted title. [obs]
- The **Scheduled** focus list shows all tasks with future start dates. Dragging tasks onto it schedules them. [obs]
- Start-date shortcuts: `⌃T` Start Today, `.` +1 day, `⇧.` +1 week, `,` −1 day, `⇧,` −1 week, `⌥,` remove start date; `k` Schedule… opens the start-date picker. [obs]
- When a task has both dates, the row shows **start (green) above due (black/red)**, e.g. "Yesterday / Sunday". [obs]
- **In Progress** = started (start date ≤ now) but not completed. It's a focus-filter and search type (`type: inprogress`). [obs/inf]
- "Duration for a task, with due date, marks a start point so those tasks can appear in Today sooner": due minus duration acts as an implicit start. [obs]
- Removing the start date from a repeating task warns that it may stop repeating. [obs]

## 5. Defer / postpone (batch)

`d` Defer… opens a popover titled "Due date of N tasks…" [obs] with:
- a two-month calendar with a date field ("11 Jul") and a Time checkbox + time field,
- quick buttons **Today ⌥1, Tomorrow ⌥2, <next weekday> ⌥3, Week ⌥4**,
- **Postpone by [N] [Days | Weeks | Months …]**,
- Save.

Selected tasks can also be dragged onto calendar dates or a drag bar to defer. [obs]

## 6. Repeating tasks

Observed options [obs] (UI strings):
- Presets: **Every Day, Every Week, Every 2 Weeks, Every Month, Every Year**, and **Work days** (uses the work days from settings).
- Custom: **Every N Days / Weeks / Months / Years**, specific weekdays ("Every Monday & Thursday").
- Monthly: on day N; **on the 1st…5th <weekday> of each month**; **last day of the month**; **last weekday**; **last weekend day**; "On the Last Day of Every N Months".
- **Repeat from completion date** (next occurrence is calculated from when you complete it, not from the due date).
- **End Repeat**: never / after N repeats / on a date (schema: `recurrenceendtype`, `recurrenceendrepeats`, `recurrenceenddate`).
- Repeating tasks show a circular-arrow checkbox in the list. [obs]
- Shortcut: ⌘; Edit Recurrence. [obs]

Behaviour on completion [obs, from the user's synced data]: 2Do **creates a completed copy** of the occurrence (shown in Done, no repeat, linked back via `ruid`) and **moves the original task's due date (and start date) forward** by the rule. The repeating task keeps its uid forever. Synthetic example: a monthly task on the 10th has completed copies for January and February 2000, and the live task is due March 10, 2000.

### Exact encoding [obs]

A rule is three integers: `recurrenceType` (sync key `repeattype`), `recurrenceValue` (`repeatval`), `recurrenceFrom` (`recurrence`).

| `recurrenceType` | Meaning | `recurrenceValue` | 2Do's own label |
|---|---|---|---|
| `0` | no repeat | — | — |
| `1`–`127` | **specific weekdays**, bitmask: Sun `1`, Sat `2`, Fri `4`, Thu `8`, Wed `16`, Tue `32`, Mon `64` (bit order Sun/Mon/Wed/Thu/Sat [obs], Tue/Fri by position [inf]) | — | "Every Mon, Thu"; `127` = "Every Day"; work days Mon–Fri = `124` |
| `0x100` (256) | every N **days** | N | 1 → "Daily", else "Every N Days" |
| `0x101` (257) | every N **weeks** | N | 1 "Weekly", 2 "Bi-weekly", else "Every N Weeks" |
| `0x102` (258) | every N **months** | N | 1 "Monthly", 2 "Bi-monthly", 3 "Every 3 Months", 6 "Semi-yearly" |
| `0x103` (259) | every N **years** | N | 1 "Yearly", else "Every N Years" |
| `0x104`–`0x10A` (260–266) | **Nth weekday of each month**: Mon, Tue, Wed, Thu, Fri, Sat, Sun | ordinal 1–5, anything else = last | "The second Tue of each month" |
| `0x10B` (267) | **last day of the month**, every N months | N | "On the last day of every month" / "…every N months" |
| `0x10C` (268) | **Nth weekday** (Mon–Fri) of each month | ordinal, else last | "The last weekday of each month" |
| `0x10D` (269) | **Nth weekend day** of each month | ordinal, else last | "The first weekend day of each month" |

- `recurrenceFrom`: **`2` = repeat from completion date**; any other value (`1` normally) = from the due date. It's also set on tasks that don't repeat, so only `recurrenceType != 0` means "repeats".
- `recurrenceEndType`: **`0` never, `1` on a date** (`recurrenceEndDate`), **`2` after N occurrences**. `recurrenceEndRepetitions` counts down the remaining occurrences; `…Orig` keeps the original N.
- For from-completion rules, the next date is computed from the completion time (or from the start date if that's later).

Port recommendation: store the rule as an **RFC 5545 RRULE** plus a `from_completion` flag. Mapping: `0x100–0x103` → `FREQ=DAILY/WEEKLY/MONTHLY/YEARLY;INTERVAL=N`; bitmask → `FREQ=WEEKLY;BYDAY=…`; `0x104–0x10A` → `FREQ=MONTHLY;BYDAY=<n><DAY>` (last = `-1`); `0x10B` → `FREQ=MONTHLY;INTERVAL=N;BYMONTHDAY=-1`; `0x10C` → `FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=n`; `0x10D` → `…BYDAY=SA,SU;BYSETPOS=n`; end type 1/2 → `UNTIL` / `COUNT`.

## 7. Alerts (alarms)

- Multiple alerts per task (⌘' Edit Alerts). [obs]
- An alert is absolute (a date/time) or relative to the due date/time. [obs] (`alarms.relativetrigger` / `absolutetrigger`)
- With no due date, a relative alert counts from the start date: "counts days from the start date and fires at the time you set". [obs]
- **Snooze** from the notification; "Mark as completed" appears first in notification actions. [obs]
- Optional repeated reminders (nag), e.g. "Every Minute / 2 / 5 / 15 mins". [obs/inf]
- Optional default ("auto") alert for tasks with a due time (`alarms.isauto`). [obs/inf]
- Alerts are delivered by a background helper even when the app is closed. [obs]
- Port: a small user service (systemd `--user` timer or a daemon) sending `libnotify` notifications with **Complete** and **Snooze** actions.

## 8. Priority and Star

- Priority **None 0 / Low 1 / Medium 2 / High 3**. Single-key shortcuts `0 1 2 3`, `=` increase, `-` decrease; ⌥⌘0–3 also work. [obs]
- Star is separate: `s` star, `⌃S` unstar, ⌥⌘S toggle. [obs]
- Row indicator at the far right: horizontal bars; **High = three red bars, Medium = two orange bars**; Low = one bar [inf]; a **yellow star** overlays the bars when starred. [obs]

## 9. Tags

- Many tags per task, shown as rounded "bubbles" in the row and as dark pills in the editor. [obs]
- Tags can be grouped into **Tag Groups** (New Tag ⌘T, New Tag Group ⌘⇧T). [obs]
- Edit Tags ⌘R, `l` Tag…, `⌥L` Untag…, `⌃⌥R` Remove all tags. [obs]
- **Tags panel** on the right (⌥⌘→ Show Tags): shows used/unused tags; click a tag icon to filter. [obs]
- **Paused tags**: assigning a paused tag hides the task ("someday" items). Pause/resume a tag in the Tags panel. Show Paused ⌘U, "Show Scheduled & Paused" ⌘⇧K. [obs]

## 10. Lists, list groups, sections, Inbox

- **Lists** are colour-coded containers (each list has a colour). [obs]
- **List groups** are headings in the sidebar (e.g. Personal, Work, Travel) holding lists. Collapsible; "Expand/Collapse all List Groups" ⇧⌘↓/↑. [obs]
- Projects appear nested under their list in the sidebar. [obs]
- **Sections**: named groups inside a list ("Housekeeping", "Small repairs"), drag to reorder, collapsible, drag tasks onto a section title to move them. [obs]
- **Inbox** (optional, GTD-style): the default collection list. Enabling it creates an "Inbox" list. [obs]
- **Default list** setting: where new tasks go (except new sub-tasks inside Project View). [obs]
- **Archive list**: archived lists and their tasks are hidden everywhere except Smart Lists and Done. [obs]
- Per-list flags: **exclude from All** (`canshowinall`), **exclude from Today** (`canshowintoday`), hidden, show-in-Done. [obs]
- Move task(s) to another list: `m` Move. ⌘L edits the list. [obs]
- Previous/next list: ⌘↑ / ⌘↓. `j` opens the List Jump Bar (jump by typing part of a name). [obs]

## 11. Focus lists (fixed views at the top of the sidebar)

| View | Shortcut | Contents |
|---|---|---|
| Inbox | ⌘0 | only if Inbox is enabled |
| **All** | ⌘1 | all unfinished tasks from lists not excluded from All |
| **Today** | ⌘2 | tasks due today; overdue tasks (setting `showOverdueInToday`); tasks whose start date has passed ("still working on") and tasks whose duration window has started (due − duration ≤ now); starred tasks if preferred, even from lists excluded from Today; checklist items inherit their checklist's dates. **Never shows completed tasks.** [obs: query builder `queryWhereClauseForDueOrStartFromDate:…includeOverDueTasks:…includeStillWorkingOn:includeStarredTasksIfPreferred:includeChecklistSubtasks:`, `ListSection` logic; exact SQL composition inf] |
| **Starred** | ⌘3 | starred tasks |
| **Scheduled** | ⌘4 | tasks with a future start date |
| **Done** | ⌘5 | completed tasks; "Tasks you complete today appear in the Done section until the day ends, whatever their due date" [obs] |

The sidebar shows counts as **`10 | 14`**: the red number is due/overdue, the grey number is the total [inf]. With a single total, only one number shows.

The Today header shows the full date ("Today Tuesday, 14 July 2026"). [obs]

## 12. Sorting and grouping

The sort bar sits above the list: **direction arrow ↓/↑** and a **sort field** menu [obs]:
`Status, Priority, List, Start Date, Due Date, Creation, Modified, Note, Title, Manually, Tag, Location, Duration`, and a **Separated** toggle.

- Sort settings are **per list and per project**. [obs] (`calendars.tasksortfield/tasksortorder`, `tasks.sortfield/sortorder`)
- **Stored sort codes** (`tasksortfield` on lists, `sortfield` on projects) [obs: cross-checked against a local dataset]:

  | Status | Priority | List | Start Date | Due Date | Creation | Modified | Note | Title | Manually | Tag | Location | Duration |
  |---|---|---|---|---|---|---|---|---|---|---|---|---|
  | 0 | 4 | 5 | 11 | **1** (default) | 13 | 15 | 2 | 6 | 7 | 9 | 10 | 14 |

  `sortorder`: 0 ascending, 1 descending [inf]. Date-based sorts (1, 11, 13, 15) produce date group headers; due-date grouping applies to 1, start-date grouping to 11. [obs: `createSectionWithDate:…sortBy:`]

- **Date group headers** (exact rules) [obs]. Measured from the start of today:

  | Condition | Header |
  |---|---|
  | No date | No Due Date / No Date |
  | Task (or project) overdue | **Overdue** (red) |
  | In Today: started task (when overdue isn't shown) or duration window started | **In Progress** |
  | Due today, time still ahead (when allowed) | **Later Today** |
  | 0 ≤ d < 1 day | **Today** |
  | 1 ≤ d < 2 days | **Tomorrow** |
  | 2 ≤ d < 3 days | **Day After Tomorrow** |
  | 3 ≤ d < 7 days | **Next 7 Days** |
  | 7 ≤ d < 14 days | **Next 2 Weeks** |
  | 14 ≤ d < 30 days | **Next 30 Days** |
  | ≥ 30 days | **Future** |
  | Future start date (start-date grouping) | **Scheduled** |

  With "Separated" off (`showingGroupedDates = false`), every distinct day gets its own header (e.g. "Thursday, 16 July") instead of the buckets. Creation/Modified sorts group by those dates the same way. Dropping a task onto a header sets its date to that bucket (drop targets). [obs]
- Each header shows the task count on the right and optionally the **total duration** in purple ("TODAY 15m"). [obs]
- Manual sort: drag; ⌃⌘↑/↓ Move Up/Down. Hold ⌘ while dragging to move between tasks without nesting. [obs]

## 13. Focus filter (the sun icon)

A per-list quick filter, toggled with ⌘J or the ☀ button. Its dropdown sets the criteria [obs]:
`Overdue, No Due Date | In Progress, Today, Tomorrow | This Week, Next 3 Days, Next 7 Days, Next 30 Days, Next 6 Months, Next 12 Months | High Priority, Starred` (multi-select). Stored per list (`calendars.focusfilterstring`). Future occurrences of repeating tasks count.

## 14. Smart lists and search

- **Find** ⌘F filters the current list; ⌥⌘F switches scope between "This List" and "All Lists". `f` opens the Task Find Bar; `/` opens the Tag Search Bar. [obs]
- **Quick Open** ⇧⌘O: a Spotlight-style popup that matches tasks and lists by name. [obs]
- **Search presets / options** (⌥⌘⇧F) [obs]: rule builder "Filter results with the following rules" (hold ⌥ for a sub-filter), presets *Next 3 Days, Next 7 Days, Next 2 Weeks, Next 4 Weeks, Without Due Date, Custom*, date range From/To, dynamic range "In the next N Days/Weeks/Months/Years", *Include Scheduled*, *Soundex* (fuzzy) matching.
- Save a search with {+} → **Smart List** (sidebar section "SMART LISTS", gear icon, own colour). [obs]
- **Search syntax** [obs]:
  - plain words match title/notes
  - `type:` values seen: `overdue`, `inprogress`, `today`, `thisweek`, `repeat`, `proj`, `chck`
  - `tags: home`, `tags: (home OR play) AND !work`
  - `lists: …`, `groups: home, travel`
  - filters combine with implicit **AND**; `OR` between filters; `!` negates (`type: today AND !starred`)
  - `{datefilter}` stands for the preset date range in an OR expression (`type: inprogress OR {datefilter}`)

**Filter prefixes** (exact regex from the app) [obs]:
`(?i)(?<!\S)(?:type:|tags:|locations:|lists:|groups:|taggroups:|!?(?:modified:|created:|completed:)|\{datefilter\})`
Tokenizer: `!+(?:\s*[^\s(),!]+)?|[^\s(),!]+|[(),]`. A trailing `AND`/`OR` is ignored (`(?i)(?<!\S)(AND|OR)\s*$`). Values are comma-separated (`groups: home, travel`).

**`type:` values** (string table next to the search parser) [obs]:

| Group | Values |
|---|---|
| Due | `overdue`, `tasksoverdue`, `duetoday`, `duetomorrow`, `duethisweek`, `tomorrow`, `thisweek` (also `today`) |
| Start | `starttoday`, `starttomorrow`, `startthisweek`, `nostart`, `inprogress`, `scheduled` |
| Due or start | `anytoday`, `anytomorrow`, `anythisweek` |
| Done | `donetoday`, `doneyesterday`, `donethisweek` |
| Priority / flags | `hiprio`, `medprio`, `lowprio`, `noprio`, `starred`, `paused` |
| Structure | `subtasks`, `alltasks`, `emptyproj`, `emptychck` (plus `proj`, `chck` from in-app examples) |
| Attributes | `repeat`, `alarms`, `duration`, `anytag`, `anylocation`, `nolocation`, `action` (`call`, `message`, `email`, `browse`, `visit`, `google`), `capturedmail` |

`modified:` / `created:` / `completed:` take a date range and can be negated with `!`. The rule builder's criteria ("CompletionIs/IsNot/Today/Week/Yesterday", "PriorityIs/IsNot Any/No/Low/Med/Hi", "ActionIs/IsNot …") compile to these tokens. [obs]

## 15. Completing tasks

- Click the round checkbox, ⌘. **Mark as Completed**, ⌥⌘. **Mark as Not Completed**. [obs]
- A completed task leaves normal lists (unless "Show completed tasks for" is set) and appears in **Done**. ⌘O Show Completed shows them in place. [obs]
- Settings [obs]: **Show completed tasks for** (Never / …), **Apply rule to sub-tasks**, **Auto-delete completed tasks** after (1 day / 1 week / 1 month / Never …), "Delay sub-tasks by 3 weeks" (see below).
- Repeating tasks: see §6.
- Completing all sub-tasks does **not** auto-complete the project: there is no such setting in the UI strings, and completing/uncompleting a child only refreshes its parent (`completeTask:updateParent:`). [inf]
- "Delay sub-tasks by 3 weeks" sits under *Completed Tasks* next to auto-delete, so it most likely keeps completed sub-tasks 3 weeks longer before auto-deletion. [inf]
- Undo ⌘Z / Redo ⇧⌘Z everywhere. [obs]

## 16. Quick Entry (global capture window)

- A system-wide hotkey opens a floating card [obs] with: checkbox + title field, a blue **list selector** tab, attribute icons (image, audio, location, calendar, repeat, alarm, duration), a tag field, **Start / Time ▶** and **Due date / Time ✓** fields, a cream Notes area with Markdown toggle, a **Task/Project/Checklist** selector, Cancel and **Save**.
- Typing a weekday prefix ("Sa") in the date field offers the next three matches ("Sat, 18 Jul in 4 days", "Sat, 25 Jul in 11 days", …). [obs]

### Typing into date, time and duration fields (exact regexes from the app) [obs]

| Input | Regex | Examples |
|---|---|---|
| Relative date (+ optional time) | `^(\d+)(days\|day\|d\|weeks\|week\|w\|months\|month\|m\|years\|year\|y\|st\|nd\|th\|rd)\s*((\d{0,2}\s*:?\s*\d{0,2})\s*((?:[ap]m?))?)\s*$` | `3d` (in 3 days), `2w`, `1m`, `1y`, `15th` (next 15th of a month), `3d 9am`, `2w 14:30` |
| Weekday (+ optional time) | `^(sunday\|sun\|monday\|mon\|tuesday\|tues\|tue\|wednesday\|wed\|thursday\|thur\|thu\|friday\|fri\|saturday\|sat)\s*(time)?$` | `fri`, `mon 9am` |
| Time | `^(0?[0-9]\|1[0-2]):?([0-5][0-9])?(\s*?[ap]\.?m?\.?)?$` | `9`, `930p`, `9:30 pm`, `12a` |
| Midnight | `(^\|[^0-9A-Za-z])(?:midnight\|(?:0?0\|12)\s*(?::\|\.)\s*0{1,2}\s*(?:a\.?m\.?)?\|12\s*a\.?m\.?\|0{3,4})(?=$\|[^0-9A-Za-z])` | `midnight`, `00:00`, `12am`, `0000` |
| Duration / offset (optionally compared) | `^([><])?\s*([0-9]+(?:[.\|,][0-9]+)?)\s*(mins\|min(ute(s)?)?\|h(our(s)?)?\|d(ay(s)?)?\|w(eek(s)?)?\|m(onth(s)?)?\|y(ear(s)?)?)` | `30 mins`, `1.5h`, `>2d` |

Note: in the relative-date regex `m` means **months**; in the duration regex `m`/`min` is checked as `mins`/`min…` first, so `30m` falls to `m(onth)` [inf]. Pick unambiguous rules for the port.
- Also available in-app: "New Quick Entry". [obs]
- macOS Services menu: "New 2Do: Selection as Title / as Note". [obs]
- On Omarchy: bind a Hyprland key to `yourapp --quick-entry`.

### Natural-language input
"2Do reads **dates, start dates, repeats, duration, priority, tags, lists and locations** from what you type and fills them in." Toggle: "Detect dates and details as you type" (separately for Quick Entry and the editor). Recognised phrases are highlighted in the title and can be undone. [obs] Date parsing uses **SwiftyChrono** (a port of chrono.js). [obs]

**Function syntax** [obs]: `list(Work)`, `tag(home)`, `action(…)`, `due(tomorrow)`, `priority(!!!)`; regexes `(?i)list\s*\(([\s\S]+?)\)`, `(?i)tag\s*\(([\s\S]+?)\)`, `(?i)action\s*\(([\s\S]+?)\)` and a generic `\s*\(([\s\S]+?)\)`. Used by Email-to-2Do subjects and accepted in titles. The title parser also matches **existing list and tag names** from a catalog (`NaturalLanguageListCatalog`, `NaturalLanguageListResolution`) rather than needing a prefix character. [obs/inf]

Other keywords seen in the parser's literals and strings [inf: not traced through the Swift parser]: `every …`, `daily/weekly/monthly/yearly`, `weekday(s)`, `for <duration>` (duration), `starting …` (start date), `high/medium/low` (priority), `starred`. A port can safely adopt: dates via chrono, `every …` / `weekly` for repeats, `for 30m` for duration, `starting fri` for start, `!`/`!!`/`!!!` or `high/medium/low` for priority, and existing list/tag names or `list(…)`/`tag(…)`. Choose your own (e.g. `#tag`, `@list`, `!`–`!!!`, `every monday`, `for 30m`) and use `chrono-node` (JS) or `dateparser` (Python) for dates.

## 17. Task editor (inline)

Double-click or ↩ on a task expands it **in place** into a card [obs]; see 03-look-and-feel for the anatomy. ⌘↩ ends editing. Direct jumps: ⌘E notes, ⌘/ dates, ⌘; recurrence, ⌘' alerts, ⌘R tags, ⌘L list, ⌘B action, ⌘G location, `x` duration. Above the card: duplicate, share, ⌘ (shortcuts), delete (red trash).

## 18. Multi-select and batch

Select multiple tasks (⌘/⇧-click, ⌘A) and apply any of: defer (`d`), schedule (`k`), move (`m`), tag (`l`), priority, star, complete, delete (⌘⌫), duplicate (⌘D), convert. [obs] "Batch…" submenu exists. [obs]

## 19. Settings worth reproducing

General: week start, **work days**, "Show weekends in red". Tasks: default due time, overdue-uses-time, show completed for, auto-delete completed, default list / Inbox, default due date for new tasks, natural-language toggles. Today & Lists: what Today includes (starred, overdue), counts. Appearance: font size (View › Font Size), colour bars. [obs]
