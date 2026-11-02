# The TUI: keys that change tasks

`tdo` (no arguments) opens the TUI. Navigation, views and `:` commands are in the help overlay
(`?`). This page covers the keys that write. Spec: §7 (creation and editing) and §8 (keys).

## Keys

| Key | What it does |
|---|---|
| `x` / `Space` | complete the selected task; on a completed task, reopen it. On a checklist or project with open items it asks first: press `x` or `Space` again to confirm, any other key cancels |
| `t` / `T` | due today / tomorrow (a timed due keeps its time) |
| `]` / `[` | due one day later / earlier (no due date: counts from today) |
| `}` / `{` | due one week later / earlier |
| `dd` | delete |
| `u` / `Ctrl-r` | undo / redo the last write of this session |
| `m` | move to another list (picker: `j`/`k`, `Enter`, `Esc`) |
| `i`, `Enter` on a task without children | edit form |
| `r` | edit form, on the Repeat field (repeating tasks only) |

Write keys act when the Tasks or Details pane has focus. With Details focused, `j`/`k`,
`gg`/`G` and `Ctrl-d`/`Ctrl-u` scroll the pane instead of moving between tasks; another task
opens at the top. They do nothing while help, `:errors`
or `:conflicts` is open.

## Projects and checklists

Like 2Do, a write on a checklist or project acts on the whole family. Completing the parent
completes every open item; reopening it reopens the completed ones; moving it to another list
and deleting it carry all items. Because completing writes many
tasks, `x` on a parent with open items shows `complete "<title>" and its <n> open items? x to
confirm` and waits; a parent whose items are all done completes at once. Cancelled items are
left as they are. `tdo op complete <uid>` refuses such a parent (exit 1) unless `--all` is
passed.

## Edit form

Fields: Title, Due, Start, Repeat (`every N <unit>`, only for repeating tasks) and Notes.
`Tab`/`Shift-Tab` (or `↓`/`↑`) move between fields, `Enter` saves, `Ctrl-s` saves from any
field, `Esc` cancels. `Ctrl-u` clears a field, `Ctrl-w` deletes a word. On Notes, `Enter`
opens `$VISUAL` / `$EDITOR` (else `vi`); save and quit to bring the text back. Pasted text goes
into the focused field as one line. Only the fields you changed are written.

Due and Start take:

| Input | Means |
|---|---|
| empty | no date |
| `today` `tod` `tomorrow` `tom` | that day |
| `3d` `2w` `1m` `1y` | from today (`m` is months; month ends clamp, Jan 31 + 1m = Feb 28) |
| `15th` | the next 15th after today |
| `fri`, `friday` | the next Friday, never today |
| `2026-11-20` | that date |
| a time after any of these, or alone (today) | `5pm`, `930a`, `9:30 pm`, `14:30`, `noon`, `midnight` |

A bare number such as `9` is rejected as ambiguous. Start dates take no time. The line under
each date shows how it was read.

## When a write is refused

The status line says why, and nothing is written:

- **Not verified yet:** the compatibility gate's reason, for example
  `deleting one item of a project or checklist is not verified yet`. See `compat/matrix.toml`.
- **`… changed on disk; reload and try again`:** a sync changed the task after you started.
  The view reloads; an open form switches to the synced version, so you see the new data
  before saving again.
- **`a sync is running; try again in a moment`:** a sync holds the vdir. The TUI does not
  wait for it, so it never freezes; press the key again once the sync is done.
- **`cannot write: …`:** the journal could not be opened (see [journal.md](journal.md)).
  The TUI still shows your tasks.

Success messages clear after five seconds. With `[sync] after_edit = true`, each write starts a
background sync ([sync.md](sync.md)).

## Not in the TUI yet

`o` / `O` (new task or item, natural-language add), `J` / `K` (manual order), `a` (alerts),
list management, and adding or removing a repeat. These keys show `not available yet`.
