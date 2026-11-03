# The TUI

`tdo` (or `tdo tui`) opens the terminal UI on the vdir from your config, or the one given with
`--vdir`. `?` shows every key.

![The Twydo TUI: views, completing a task, a checklist, the edit form and a list](demo/tui.gif)

## The screen

```
╭ Views ───────╮╭ Today · Monday 2 Nov ─────────────────╮╭ Details ──────────────╮
│ ◷ Today  2│3 ││ OVERDUE                             1 ││ Renew library books   │
│   All   2│10 ││ ▌○ Renew library books    Yesterday  ││                       │
│   Scheduled 1││                                      ││ List    Inbox         │
│   Done      2││ TODAY                               1 ││ Due     Yesterday     │
│ ─ Lists ──── ││ ▌↺ Water plants    weekly   Today    ││                       │
│ ▌ Bills     6││                                      ││                       │
│ ▌ Inbox   1│2││                                      ││                       │
│ ▌ Personal1│2││                                      ││                       │
│ ─ Smart lists││                                      ││                       │
│   Next 3 Days││                                      ││                       │
╰──────────────╯╰──────────────────────────────────────╯╰───────────────────────╯
 NORMAL Tasks · j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help   Today·1/2
```

- **Views** (left): the views, your lists with their colour bars, and smart lists. `2│3` is
  "due today or overdue │ all"; see [tasks.md](tasks.md#views-and-date-groups) for what each view
  shows. List groups come from `[[groups]]` in [config.md](config.md).
- **Tasks** (middle): the current view under date-group headers. `○` a task, `↺` a repeating
  task with its rule, `[3]` a checklist or project with 3 items. The bar on the left is the
  list's colour. Overdue rows are red.
- **Details** (right): the selected task's list, dates, repeat, alerts, warnings, its items (on a
  parent) and notes. It appears when the terminal is at least 110 columns wide.
- **Status line**: the mode (`NORMAL`, `INSERT`, `COMMAND`), the focused pane, key hints or the
  last message, and the position in the view. An indicator appears when files could not be read
  (`:errors`).

The focused pane has a bold blue border. The sidebar takes a fifth of the width (24 to 40
columns), or a third below 60 columns.

Colours come from your terminal's 16-colour palette, so the TUI follows your terminal theme and
re-colours live when it changes. Only the list bars use the lists' own (true) colours.

The TUI reloads when files in the vdir change, for example after a sync, and every minute, so
"today" and the groups stay current.

## Keys

### Moving around

| Key | What it does |
|---|---|
| `j` / `k`, `gg` / `G`, `Ctrl-d` / `Ctrl-u` | move down / up, to the top / bottom, half a page. With Details focused they scroll it instead |
| `h` / `l`, `Tab` | focus the previous / next pane |
| `1`–`5` | Today, All, Scheduled, Done, Next 3 Days |
| `Enter` in the sidebar | open the view or list |
| `Enter` on a parent | expand or collapse its items (All and lists) |
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
| `m` | move to another list (picker: `j`/`k`, `Enter`, `Esc`) |
| `i`, or `Enter` on a task without items | edit form |
| `r` | edit form, on the Repeat field (repeating tasks only) |

On a checklist or project, a write acts on the whole family; see
[tasks.md](tasks.md#completing-reopening-moving-deleting).

### Commands

| Command | What it does |
|---|---|
| `:list <name>` | open a list by name (or collection id), also a hidden one |
| `:sync` | sync now (needs `[sync]` in the config; [sync.md](sync.md)) |
| `:sync! <list>` | sync and accept that a list was emptied on purpose ([sync.md](sync.md)) |
| `:conflicts` | show sync conflicts; `l` keeps the local version, `r` the remote one |
| `:errors` | list files that could not be read |
| `:q` | quit |

## The edit form

Fields: Title, Due, Start, Repeat (only on repeating tasks) and Notes.

- `Tab` / `Shift-Tab` (or `↓` / `↑`) move between fields; `Enter` saves, `Ctrl-s` saves from any
  field, `Esc` cancels.
- `Ctrl-u` clears a field, `Ctrl-w` deletes a word. Pasted text goes into the focused field as
  one line.
- On Notes, `Enter` opens `$VISUAL` or `$EDITOR` (else `vi`); save and quit to bring the text
  back.
- Repeat reads `every _ months`: type the interval.
- Only the fields you changed are written.

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

A bare number such as `9` is rejected as ambiguous. Start dates take no time. The line under each
date shows how it was read (`→ Fri 6 Nov 10:00`).

## When a write is refused

The status line says why, and nothing is written:

- **`… is not verified yet`**: Twydo has not verified this write against 2Do on the iPhone, for
  example `weekdays-set repeats are not verified yet`. See
  [compatibility.md](compatibility.md#verified-mode).
- **`… changed on disk; reload and try again`**: a sync changed the task after you started. The
  view reloads; an open form switches to the synced version, so you see the new data before
  saving again.
- **A sync is running.** A sync holds the vdir while vdirsyncer runs. The save waits up to two
  seconds, so an ordinary sync finishes unnoticed. If it is still running after that, the status
  line asks `sync running for 2m 14s — stop it and save? (y/n)`:
  - `y` stops the sync and saves (the message ends in `· sync stopped`). The stopped sync counts
    as a failed one ([sync.md](sync.md#stopping-a-sync)): it backs off, and your edit stays
    pending until the next sync that works sends it. Any sync can be stopped this way, whether the
    TUI, the timer or a `tdo sync` started it. If it does not let go within ten seconds, nothing is
    written: `not saved; the sync did not stop`.
  - `n` or `Esc` writes nothing: `not saved; the sync is still running`. An open edit form stays
    open with what you typed; for a one-key action, press the key again later.

  Other keys do nothing while the question is open. When the TUI cannot tell which sync holds the
  vdir, it says `a sync is running; try again in a moment` instead.
- **`cannot write: …`**: the operation journal could not be opened
  ([architecture.md](architecture.md#the-operation-journal)). Your tasks are still shown.

Success messages clear after five seconds. With `[sync] after_edit = true` (the default when
sync is on), each write starts a background sync.

## Not in the TUI yet

`o` / `O` (add a task or item, with natural-language input), `J` / `K` (manual order), `a`
(alerts), list management, and adding or removing a repeat. These keys show `not available yet`;
the help marks them `(after gate)`.
