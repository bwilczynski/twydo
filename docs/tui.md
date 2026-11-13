# The TUI

`tdo` (or `tdo tui`) opens the terminal UI on the vdir from your config, or the one given with
`--vdir`. `?` shows every key.

![The Twydo TUI: completing a task, adding one and turning it into a project with items, adding checklist items, editing a task's due date in the card, and the help](demo/tui.gif)

## The screen

```
╭ Views ───────╮╭ Today · Monday 2 Nov ─────────────────╮╭ Details ──────────────╮
│ ▌ Inbox   1│2││ OVERDUE                             1 ││ Renew library books   │
│ ◷ Today  2│3 ││ ▌○ Renew library books    Yesterday  ││                       │
│   All   2│10 ││                                      ││ List    Inbox         │
│   Scheduled 1││ TODAY                               1 ││ Due     Yesterday     │
│   Done      2││ ▌↺ Water plants    weekly   Today    ││                       │
│ ─ Lists ──── ││                                      ││                       │
│ ▌ Bills     6││                                      ││                       │
│ ▌ Personal1│2││                                      ││                       │
│ ─ Smart lists││                                      ││                       │
│   Next 3 Days││                                      ││                       │
╰──────────────╯╰──────────────────────────────────────╯╰───────────────────────╯
 NORMAL Tasks · j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help   Today·1/2
```

- **Views** (left): the focus lists (your Inbox, then the views), your other lists with their
  colour bars, and smart lists. The Inbox is the `default_list` from [config.md](config.md),
  else the list named Inbox; without either, the focus lists start at Today. `2│3` is
  "due today or overdue │ all"; see [tasks.md](tasks.md#views-and-date-groups) for what each view
  shows. List groups come from `[[groups]]` in [config.md](config.md).
- **Tasks** (middle): the current view under date-group headers. `○` a task, `↺` a repeating
  task with its rule, `[3]` a checklist or project with 3 items. The bar on the left is the
  list's colour. Overdue rows are red. Completed titles are struck through, including
  selected rows, projects/checklists and their completed items.
- **Details** (right): the selected task's list, dates, repeat, alerts, warnings, its items (on a
  parent) and notes. It appears when the terminal is at least 110 columns wide.
- **Status line**: the mode (`NORMAL`, `INSERT`, `COMMAND`), the focused pane, key hints or the
  last message, and the position in the view. An indicator appears when files could not be read
  (`:errors`). With `[sync]` it shows the sync state (`synced 3m ago · 2 pending`, conflicts,
  failures; the count includes edits queued during a sync); while automatic sync is off it starts
  with `sync off` in yellow, whatever the pending count ([sync.md](sync.md#when-edits-sync)).
  An animated spinner and `Syncing…` replace the last-sync age during manual and background
  syncs, including runs started outside this TUI. When the run ends, the usual status returns:
  `synced just now`, then `synced 1m ago`, and so on after a successful sync. Errors, conflicts,
  and pending counts remain visible. Editing or entering a command does not clear sync activity.

The focused pane has a bold blue border. The sidebar takes a fifth of the width (24 to 40
columns), or a third below 60 columns.

Colours come from your terminal's 16-colour palette, so the TUI follows your terminal theme and
re-colours live when it changes. Only the list bars use the lists' own (true) colours.

The TUI shows what the daemon holds ([architecture.md](architecture.md#the-daemon)). It starts
the daemon when none runs, and reloads when the daemon says the data changed: an edit, a sync
(whoever started it), or a change made outside. Every minute it moves its clock on, so
"today" and the groups stay current.

**The daemon.** When the TUI loses the daemon (it was stopped or crashed) the status line says
`daemon not running: <reason>; retrying`; the TUI reconnects by itself and says
`daemon restarted` when it is back. Edits you make meanwhile are refused with the same
reason. If no service unit is installed, the status line says once per run `daemon won't start
at login; run tdo daemon install` (turn it off with `[daemon] unit_warning = false`,
[config.md](config.md#daemon)). `tdo was upgraded; restart it` means a newer `tdo` replaced the daemon; quit and
start `tdo` again (the TUI does not retry then). After you change `timezone`, `work_days`,
`vdir`, `state_dir`, `[writes]` or `[sync]` in the config, it says
`restart the daemon (tdo daemon stop) and reopen tdo`; other settings apply live. If a save
gets no answer within 8 seconds, it says
`daemon did not answer; the save may still apply — check before retrying`. (`tdo op status
<request-id>` looks up a request from the command line; see [sync.md](sync.md#stopping-a-sync).)

## Keys

### Moving around

| Key | What it does |
|---|---|
| `j` / `k`, `gg` / `G`, `Ctrl-d` / `Ctrl-u` | move down / up, to the top / bottom, half a page. With Details focused they scroll it instead |
| `h` / `l`, `Tab` | focus the previous / next pane |
| `0` | the Inbox, even when it is hidden or archived (like `:list`) |
| `1`–`5` | Today, All, Scheduled, Done, Next 3 Days |
| `Enter` in the sidebar | open the view or list |
| `o` / `i` in the sidebar | create / edit a list |
| `Enter` on a parent | expand or collapse its items, in every view |
| `/`, then `n` / `N` | search the current view by title; `Enter` keeps the filter, `Esc` clears it |
| `?` | help; `Esc` closes it |

### Changing tasks

These act when the Tasks or Details pane has focus.

| Key | What it does |
|---|---|
| `x` / `Space` | complete the selected task; on a completed task, reopen it. On a checklist or project with open items it asks first: press `x` or `Space` again to confirm, any other key cancels |
| `t` / `T` | due today / tomorrow (a timed due keeps its time) |
| `]` / `[` | due one day later / earlier (no due date: counts from today) |
| `}` / `{` | due one week later / earlier |
| `dd` | delete |
| `u` / `Ctrl-r` | undo / redo the last write of this session |
| `m` | choose a list or nested project/checklist destination; `j`/`k` or arrows navigate, typing filters (`/` starts a filter with j/k), `Enter` chooses, `Esc` cancels. Top-level list moves preserve schedules; membership changes require dedicated evidence |
| `>>` / `<<` | indent an open leaf under the nearest preceding eligible parent / outdent into the same list, in the Tasks pane of list views only. Candidate membership operations remain refused by the shipping matrix |
| `o` / `O` | new task below / above the selected one ([Adding tasks](#adding-tasks)) |
| `J` / `K` | move a nested checklist/project item after/before the next/previous visible sibling in the Tasks pane (requires ordering compatibility evidence) |
| `a` | new item at the end of the selected project or checklist; on a plain task, asks whether to make it one first |
| `ct` / `cp` / `cc` | change the selected task's type to task / project / checklist; any other second key cancels. A project with items can't be converted, as on the iPhone: the write is refused with the reason. The type it already has writes nothing (a plain task with items shows as a project, but `ct` on it says it is already a task) |
| `i`, or `Enter` on a task without items | edit card |
| `r` | edit card, on the Repeat field |

On a checklist or project, a write acts on the whole family; see
[tasks.md](tasks.md#completing-reopening-moving-deleting).

### Commands

| Command | What it does |
|---|---|
| `:list <name>` | open a list by name (or collection id), also a hidden one |
| `:sync` | sync now; behind a sync that is already running, it waits and then runs (needs `[sync]` in the config; [sync.md](sync.md)) |
| `:sync on` / `:sync off` | switch automatic sync on or off ([sync.md](sync.md#when-edits-sync)); on starts a sync when edits are unsynced; any other argument shows the usage |
| `:sync! <list>` | sync and accept that a list was emptied on purpose ([sync.md](sync.md)) |
| `:discover` | find lists created on another device (local folders only), then sync, and reload the lists ([sync.md](sync.md#new-lists-from-other-devices)); any argument shows the usage |
| `:conflicts` | show sync conflicts; `l` keeps the local version, `r` the remote one |
| `:errors` | show unreadable files and sync errors; j/k scroll |
| `:lists` | manage all lists, including hidden and archived lists, or resume a pending list save |
| `:list-backups` | browse completed raw collection backups |
| `:export-list-backup <id> <directory>` | export exact raw collection data into a new directory outside the vdir |
| `:forget-list [list]` | choose a list, confirm it was deleted remotely, preserve its local copy, and refresh the synced list set |
| `:recover` | browse preserved lists; Enter previews tasks and notes, r chooses a destination |
| `:recover <backup-id> <destination-list>` | preview and confirm recovery into a named existing list |
| `:q` | quit |

In the sidebar, `o` opens a new list card and `i` edits the selected ordinary list.
Use Tab to move between name, color and actions. Color accepts `#RRGGBB`; Left/Right
cycles the 11 preset colors. **Save to CalDAV** applies immediately, even when automatic
sync is off. Creating a list is verified in both directions.
Renaming and recoloring an existing list saves to CalDAV in both write modes. It is
best-effort: the server offers no atomic precondition, so a save can overwrite a
simultaneous name or color change made on another device. The card says so; the result
is read back and confirmed.
A refusal retains the card input. Pending saves offer **Check pending save** and
**Keep server values**; these inspect the recorded target without repeating a write.

**Archive on this desktop** is a separate action: it saves no pending name/color edits
and works offline. Unfinished or unreadable tasks require confirmation. Use `:lists`
to unarchive. An archived selected list returns to Today. Other running Twydo windows
reload the saved list preferences while retaining active card input.

Sidebar `dd`, or the deletion action in a list card, requests a fresh remote inventory
and a private backup before confirmation. The backup includes server-only resources,
unparseable content, local files, collection properties and relevant journal/conflict
evidence. An incomplete or changing inventory is refused. The confirmation defaults to
cancel (Enter, Esc and `n` cancel; only `y` deletes) and states that deletion is
**best-effort**: the server offers no protection against concurrent remote edits, so a
change made on another device at the same moment can be lost. `y` revalidates the list
against the backup and deletes it from CalDAV and all devices. If it changed since the
backup the deletion is refused with the reason; close the card and press `dd` again for a
fresh backup and preview. On success the list leaves the sidebar (an open list returns to
Today) and the message names the backup kept. The backup stays exportable with
`:export-list-backup`; restoring one into CalDAV is not available yet (#75). The local copy
is preserved like a forgotten list, so it also appears in `:recover`. An interrupted
deletion is finished or released from the card's pending-save choices.

Use `:list-backups` and `:export-list-backup` to inspect/export these raw snapshots.
They are separate from task undo and the existing `:recover` retirement backups.
Collection restore and acceptance of a vanished pending-create/edit target remain pending
the lifecycle checkpoint; a backup export preserves every resource in the meantime.

Run `:forget-list` to choose a list by name with j/k or the arrow keys, then Enter.
The chooser includes hidden, archived, and empty lists. Duplicate names show collection
IDs beside them; unnamed lists use their IDs. Lists with a recorded Not Found sync
error show `· Not Found`; this is the last sync report, not proof of remote deletion.
Esc cancels the chooser. Selecting a list opens a compact confirmation sized to its
text; preserving and removing the local list still requires `y`.

A missing-list sync error points to `:forget-list` to choose a list. The errors panel
and CLI sync report also include the local name, collection id, and direct
`:forget-list <id>` shortcut. A Not Found response alone does not prove a list was deleted:
only confirm if you deleted it on another device. Both forgetting and recovery require
`y`; `n` or Esc cancels without writing. List arguments accept a collection id or unique
display name, including spaces. Recovery always needs an existing destination, for example
`:recover 20261105T120000Z-ab12cd34 Personal Tasks`.

The recovery preview shows the preserved local snapshot. Historical conflict alternatives
remain in the backup; they are not selected or merged automatically. Recovery uses the
existing compatibility gate and refuses an unsupported task set without a partial import.
The backup remains after recovery. `u` undoes a recovery and Ctrl-r redoes it; repeating
`:recover` cannot overwrite later edits or import the same backup into another list.
See [deleted lists and recovery](sync.md#deleted-lists-and-recovery) for storage and sync details.

## Planned and completed work

All and ordinary lists always include future-start tasks under **FUTURE** and completed tasks
under **DONE** at the bottom for the last 30 calendar days including today. Older completions
and items without a completion date remain in the dedicated Done view; nothing is deleted.
Empty sections are omitted, and Done rows sort newest completion
first. These sections remain with title/creation sorting. There are no visibility controls.
Today, Scheduled, Done and Next 3 Days keep their own membership rules.

Sidebar counts describe unfinished work, including future-start tasks; completed rows do not
increase them. The dedicated Done view shows its completed total. A child in a different
active/future-start/completed section from its parent appears separately with parent context;
an open child with no due or start date counts as being in its open parent's section.
Otherwise `Enter` expands or collapses the family's items as before.
Completing a task or item keeps the cursor on the same row, selecting the next task or
item when one follows, rather than following the completed task into Done. At the end of open
work, it selects the previous open row if one remains; otherwise it clamps to the remaining
rows. Reopening a completed child expands its ancestors so the reopened task stays selected.

## Adding tasks

`o` opens an empty row below the selected task, `O` above it. Type the title straight into the
list; the status line shows where the task will go:
`New task in Inbox · Enter save  Tab more  Esc cancel`.

| Where the cursor is | The new task |
|---|---|
| a list | goes into that list |
| Today | goes to the default list, due today |
| All, Scheduled, Done, Next 3 Days | goes to the default list, with no dates |
| an item of a project or checklist (any view) | becomes another item of the same parent; in Today a project item is due today, a checklist item has no dates |

The default list is `default_list` from the config ([config.md](config.md)), else the list named
Inbox, else the first list.

On an item, `o`/`O` save immediately after/before that sibling in the parent’s shared
`DisplayOrder` sequence, including when Tab opens the card. The saved position uses the same
DisplayOrder operation for recurring and non-recurring families. Select **Manually** on
the phone to display it there. Creating the new item must also pass the existing creation
checks, as when using `a` to append. Standalone task placement
continues to follow the view’s sort.

On the row:

- `Enter` saves. The cursor goes to the new task; when the view does not show it (a task
  without dates added in Scheduled, say), the status line says `Added "…" to Inbox` and the
  cursor stays. `Enter` on an empty row closes it.
- Inside a project or checklist, `Enter` saves and opens the next item's row at the end of
  that parent's visible items, so you
  can type a whole checklist in one go. Each saved item is its own write and undo step.
- `Tab` opens the [edit card](#the-edit-card) for the new task, keeping the title.
- `Esc` closes the row and writes nothing. Items already saved by `Enter` stay.
- `Backspace`, `Ctrl-u` (clear) and `Ctrl-w` (delete a word) edit the title; a paste goes in as
  one line.

`a` on a project or checklist opens an item row at the end of its items, expanding it. On an
item it adds another item at the end of the same parent. On a plain task it asks first:
`Add items: make it a (p)roject or (c)hecklist?`. `p` or `c` converts the task (one write) and
then opens the item row; any other key cancels. If the conversion is refused, the status line
says why and no row opens.

If a save is refused, the row stays with your text and the status line shows why. When the
parent was deleted meanwhile (on the phone, say), the row closes instead: there is nothing left
to add items to.

## The edit card

The task's row opens in place into a card; the rows below move down, and the list and the
Details pane stay in view. The view scrolls so the whole card shows.

```
│ LATER TODAY                                     1 │
│ ╭───────────────────────────────────────────────╮ │
│ │ Call the dentist▏                             │ │
│ │ ▌Inbox ▸   Task                               │ │
│ │ Due     2026-11-04 15:00  → Wed 4 Nov 15:00   │ │
│ │ Start   —                                     │ │
│ │ Repeat  none                                  │ │
│ │ Notes   —                                     │ │
│ ╰ Ctrl-s save · Tab next · Esc cancel ──────────╯ │
│ TODAY                                           1 │
```

Fields, in `Tab` order:

| Field | Input |
|---|---|
| Title | text; must not be empty |
| List | the list, in its colour. `Enter` opens the list picker (as `m`); the choice is saved with the rest, as a move |
| Type | Task, Project or Checklist: `Space` or `l` next, `h` previous. Saving converts the task ([tasks.md](tasks.md)) |
| Due, Start | a date (below) |
| Repeat | a typed rule (below); `none` when the task does not repeat |
| Notes | `Enter` opens `$VISUAL` or `$EDITOR` (else `vi`); save and quit to bring the text back |

An item of a project has no List (it lives in its parent's list). An item of a checklist shows
only Title and Notes: checklist items have no dates or repeat of their own, and `r` on one says
so.

- `Tab` / `Shift-Tab` (or `↓` / `↑`) move between fields; `Enter` saves (on List and Notes it
  opens the picker or the editor), `Ctrl-s` saves from any field.
- `Esc` cancels: nothing is written, whatever you changed (the list too).
- `Ctrl-u` clears a field, `Ctrl-w` deletes a word. Pasted text goes into the focused field as
  one line.
- Only the fields you changed are written, as one save: the changes and the move are written
  together or not at all, and `u` undoes them in one step. If any part is refused, nothing is
  written, and the card stays open with what you typed and the reason.

When the Tasks pane is narrower than 40 columns or too short for the card, the same card opens
centred over the screen instead, titled `Edit task`.

Due and Start take:

| Input | Means |
|---|---|
| empty | no date |
| `today` `tod` `tomorrow` `tom` | that day |
| `3d` `2w` `1m` `1y` | from today (`m` is months; month ends clamp: Jan 31 + 1m = Feb 28) |
| `15th` | the next 15th after today |
| `fri`, `friday` | the next Friday, never today |
| `2026-11-20` | that date |
| a time after any of these, or alone (today) | `5pm`, `930a`, `9:30 pm`, `14:30`, `noon`, `midnight` |

A bare number such as `9` is rejected as ambiguous. Start dates take no time. The text beside
each date shows how it was read (`→ Fri 6 Nov 10:00`).

Repeat takes a rule, then any modifiers:

| Input | Repeats |
|---|---|
| empty, `none` | never (on a repeating task: removes the repeat) |
| `daily`, `3d`, `every 3 days` | every N days |
| `weekly`, `biweekly`, `2w`, `every 2 weeks` | every N weeks |
| `monthly`, `3m`, `every 3 months` | every N months, on the due date's day |
| `yearly`, `2y`, `every 2 years` | every N years, on the due date |
| `mon thu`, `mon,thu`, `every mon and thu`, `workdays` | on those weekdays (`workdays` is the config's `work_days`, default Monday to Friday) |
| `last day`, `last day every 2m` | on the last day of the month |
| `2nd tue`, `last fri` | on that weekday of each month |
| `1st weekday`, `last weekend day` | on that workday (Monday to Friday) or weekend day of each month |
| modifiers: `after done`, `until 2027-06-01`, `10 times` | from the completion date; until a date; that many times |

Verified mode now supports plain daily/yearly repeat changes, captured monthly/weekly checklist
repeat changes, monthly checklist completion/reset, and narrowly scoped until completion and
reopening. The [compatibility table](compatibility.md#what-is-verified-today) gives the exact
shapes. For plain date-only monthly day 1–28 checklists (interval 1, due anchored, no end,
start, alerts or delay), the new-task card can create a repeating checklist, `cc` converts an
existing monthly task, `a` adds items, and the edit card saves titles/notes. Date keys and the
card can also move an open parent's date-only due to day 1–28 when its plain open/completed
items are undated or carry that parent's date. The source date must also be day 1–28.
Inherited item-date copies are removed; item completion state is preserved. Cancelled or
independently dated/recurring items, start/time/interval edits and other type changes stay
refused. The [scheduling audit](reference/evidence.md) distinguishes
the enabled due edit from the unverified interval candidate. A mixed save containing a refused field
retains all typed text.

`dd` on a selected checklist/project item removes only that item, including a completed or
last item and an item under a completed parent. Completed checklist items are selectable in
the Done section of All or an ordinary list; completed project items are also in Done. Empty parents remain visible in their eligible views and keep their type,
state, dates and repeat rules. Item deletion supports recognized recurring families and
independently recurring project children; it does not advance recurrence. `u`/`Ctrl-r`
restore/redo exact item bytes and identities; a changed parent, sibling or membership makes
stale inverses refuse safely. Saves during sync retain the same strict queue preconditions.
See [item behavior](tasks.md#completing-reopening-moving-deleting) for structural refusals.

`dd` on a selected checklist/project parent removes the whole flat family, including an empty
parent, a completed or cancelled family, and a recurring parent with independently recurring
items. Completed families are selectable in Done. The removal is one journaled operation, and one `u`
restores the parent and items exactly; unrelated resources and history outside the family are untouched.
Ambiguous or duplicate links, cross-list items, nested families, plain-task parents, unknown
types and unrecognized rules are refused without writes. Recurrence execution and field-edit
permissions are not required.

The field takes no day of the month: monthly and yearly repeats follow the due date, so set the
day in Due. While the field is empty, the text beside it lists examples; otherwise it shows the
rule in 2Do's words (`→ The second Tue of each month · from due date`). On a task that repeats,
an emptied field (or `none`) reads `→ none (removes the repeat)`: saving removes the rule. A task that repeats
starts with its rule in this syntax, so changing it is editing text. A repeat needs a due date:
set one on a task without a due date and it becomes due today (the preview ends in
`· due today`).

A few rules 2Do can store cannot be typed here (monthly on a fixed day of the month other than
the due date's, for example). On such a task the field starts empty, the text beside it names
the rule and ends in `(kept)`, and saving leaves the rule alone unless you type a new one, or
`none` to remove it.

## When a write is refused

Title/notes-only saves use a separate compatibility capability, including on
completed tasks and recurring checklist/project families. They leave dates,
recurrence, alarms, lifecycle and metadata intact. Other changed fields retain
their own gates: a mixed save with a refused field writes nothing and keeps the
typed card input. See the [text-edit audit](reference/evidence.md).

The status line says why, and nothing is written:

- **`… is not verified yet`**: Twydo has not verified this write against 2Do on the iPhone,
  for example a recurring checklist's interval edit or a project repeat change. See
  [compatibility.md](compatibility.md#verified-mode).
- **`… is not supported by 2Do on the iPhone`**: the repeat is one of the three kinds 2Do 3.91
  misreads (last day, last weekday, Nth weekend day). Twydo never writes them; pick another repeat.
- **`… changed on disk; reload and try again`**: a sync changed the task after you started. The
  view reloads; an open card switches to the synced version, so you see the new data before
  saving again.
- **Not saved: "…" changed on another device.** You saved while a sync was running, so the
  edit was queued and shown at once (below). When the sync ended, the task had changed on
  another device, so the edit was not written. The line is shown once, whichever of the TUI or
  the sync noticed; an edit that fails any other check says `Not saved: "…": <reason>`.
- **`cannot write: …`**: the operation journal could not be opened
  ([architecture.md](architecture.md#the-operation-journal)). Your tasks are still shown.

A save made while a sync is running is not refused and does not wait: it is queued, the message
ends in `· queued (a sync is running)`, and the task shows as saved with a muted `(queued)` after
its title. The queued edits count in the status line's `N pending`. They are written when the
sync ends (the daemon keeps the queue, so it survives closing the TUI); `u` on a queued save
cancels it before anything is written. There is no question about stopping the sync. List
operations such as `:recover` run as jobs on the daemon after a running sync, so they wait
their turn instead of failing.

If a list save was interrupted (the daemon stopped in the middle of it), the daemon still starts.
The list card offers to finish it, and the status line says why other edits wait; the journal
is recovered once the list save is finished.

Success messages clear after five seconds. Writes sync in batches: a background sync starts
once `batch_size` edits are unsynced and edits have paused for 3 seconds, or `max_wait_secs`
after the oldest one, so quick writes in a row (closing checklist items one by one) are not
interrupted by their own sync. A save made while a sync is running is queued (see above).
Quitting starts no sync; the daemon sends the last edits
([sync.md](sync.md#when-edits-sync)).

## Reordering items

In the Tasks pane, `J` moves a nested item after its next visible sibling and `K` before
its previous visible sibling. At either end, a message is shown and nothing is saved.
Selection follows the moved item. Moves stay within the same parent and displayed section;
independently shown project items in flat date views and detached children cannot be moved
with these keys. Sidebar/Details focus and overlays do not issue moves.

With a search hiding siblings, the move is relative to the next visible sibling in the full
saved sequence. For example, A,B,C,D with B hidden becomes B,C,A,D when A moves past C.
Hidden siblings keep their relative order. Details uses the resulting sibling sequence.
Each move is one undo step; a move during sync is queued and appears immediately, like other
edits. Refused stale queued moves are reported through the normal queue feedback.

Reordering works within recurring and non-recurring checklists/projects, including dated
families and families with completed/cancelled siblings. It changes only shared item ranks
and modification timestamps; recurrence, dates, alarms and completion state are preserved.
Select **Manually** in 2Do to see the saved order there. Twydo does not change that phone
setting. The saved `o`/`O` position uses the same ordering operation, while creation itself
keeps its existing compatibility checks. Details:
[ordering compatibility](compatibility.md#item-reordering).

## Not in the TUI yet

Natural-language input on the new row (dates typed with the title), alerts and list management.

Completed history tasks and flat history families can be reopened with `x` or
`Space` in Done. A family reopen changes every completed member in one undo step;
open/cancelled children stay unchanged. It keeps dates and alerts and leaves the
live recurring series untouched. Individual detached history children remain
refused, including after their parent has reopened. See
[detached history reopening](tasks.md#detached-history-reopening) for the exact boundary.

### Candidate parent membership

The destination picker distinguishes list IDs from parent UIDs and excludes the
selected leaf, its existing parent, nested/closed parents and parents of unknown
type. It has no new-list creation action. Moving a parent to another list keeps
the existing whole-family operation. Moving a leaf into a parent, or an item into
a list, uses the separate relationship operation.

Checklist inbound date loss asks `y` before writing; `n` or Escape cancels and
keeps selection. A reload cancels that confirmation so a new destination must be
chosen against current data. Unsupported start/time, recurrence or alarm cleanup
is refused. Outdenting a checklist item produces an undated plain task.

These controls are implemented for synthetic candidate verification. **Shipping
membership writes remain refused in both write modes** until the dedicated
[evidence checklist](reference/evidence.md) is approved
and completed. A refused write displays the gate reason and leaves task bytes
unchanged. This does not enable yank/paste or list management.
