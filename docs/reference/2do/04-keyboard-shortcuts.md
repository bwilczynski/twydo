# 04 — Keyboard shortcuts

Taken from the app's main menu [obs]. 2Do relies heavily on
**single-key shortcuts** when the task list has focus (no modifier). Keep those, because they're
most of the feel.

Linux mapping suggestion: **⌘ → Ctrl, ⌥ → Alt, ⌃ → Ctrl+Shift** (Super belongs to Hyprland).
Where that collides (e.g. ⌃⌘N → Ctrl+Ctrl), use the "Linux" column.

## Single-key (task list focused, task(s) selected)

| Key | Action |
|---|---|
| `t` | Due Today |
| `y` | Due Tomorrow |
| `w` | Due This Weekend |
| `e` | Due Time… |
| `]` / `⇧]` | Due date +1 day / +1 week |
| `[` / `⇧[` | Due date −1 day / −1 week |
| `.` / `⇧.` | Start date +1 day / +1 week |
| `,` / `⇧,` | Start date −1 day / −1 week |
| `d` | Defer… (date popover, postpone by N) |
| `k` | Schedule… (start date) |
| `x` | Duration… |
| `0` `1` `2` `3` | Priority None / Low / Medium / High |
| `=` / `-` | Increase / decrease priority |
| `s` | Star |
| `l` | Tag… |
| `m` | Move (to list/project) |
| `c` | Collapse project |
| `p` | Perform action (call/email/…) |
| `u` | Open links |
| `f` | Task find bar |
| `/` | Tag search bar |
| `j` | List jump bar |

## With modifiers

| macOS | Linux | Action |
|---|---|---|
| ⌘N | Ctrl+N | New Task |
| ⌃⌥N | Ctrl+Shift+Alt+N | New Sub-Task |
| ⌥⌘N | Ctrl+Alt+N | New Project |
| ⌃⌘N | Ctrl+Alt+C (custom; the rule would collide with New List) | New Checklist |
| ⌥⌘⇧N | — | New Project from Selection |
| ⌃⌘⇧N | — | New Checklist from Selection |
| ⌘⇧N | Ctrl+Shift+N | New List |
| ⌘T / ⌘⇧T | Ctrl+T / Ctrl+Shift+T | New Tag / New Tag Group |
| ⌃⌥⌘N | — | New Window |
| ⌘. | Ctrl+. | Mark as Completed |
| ⌥⌘. | Ctrl+Alt+. | Mark as Not Completed |
| ⌘D | Ctrl+D | Duplicate |
| ⌘⌫ | Delete / Ctrl+Backspace | Delete |
| ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z | Undo / Redo |
| ⌘↩ | Ctrl+Enter | End task editing |
| ⌘E | Ctrl+E | Edit Note |
| ⌘/ | Ctrl+/ | Edit Dates |
| ⌘; | Ctrl+; | Edit Recurrence |
| ⌘' | Ctrl+' | Edit Alerts |
| ⌘R | Ctrl+R | Edit Tags |
| ⌘L | Ctrl+L | Edit List |
| ⌘B | Ctrl+B | Edit Action |
| ⌘G | Ctrl+G | Edit Location |
| ⌃T | Ctrl+Alt+T (Ctrl+Shift+T is New Tag Group) | Start Today |
| ⌥, | Alt+, | Remove Start Date |
| ⌥[ | Alt+[ | Remove Due Date |
| ⇧⌘[ | Ctrl+Shift+[ | Remove Due Time |
| ⌃S | Ctrl+Shift+S | Unstar |
| ⌥⌘S | Ctrl+Alt+S | Toggle Starred |
| ⌥⌘0…3 | Ctrl+Alt+0…3 | Priority None…High |
| ⌥L | Alt+L | Untag… |
| ⌃⌥R | Ctrl+Alt+R | Remove all tags |
| ⇧⌥⌘1 / 2 / 3 | Ctrl+Alt+Shift+1/2/3 | Convert to Task / Project / Checklist |
| ⌃⌘↑ / ⌃⌘↓ | Alt+↑ / Alt+↓ | Move task up / down (manual sort) |
| ⌘0 | Ctrl+0 | Inbox |
| ⌘1 | Ctrl+1 | All |
| ⌘2 | Ctrl+2 | Today |
| ⌘3 | Ctrl+3 | Starred |
| ⌘4 | Ctrl+4 | Scheduled |
| ⌘5 | Ctrl+5 | Done |
| ⌘↑ / ⌘↓ | Ctrl+↑ / Ctrl+↓ | Previous / next list |
| ⇧⌘↑ / ⇧⌘↓ | Ctrl+Shift+↑/↓ | Collapse / expand all list groups |
| ⌥⌘↑ / ⌥⌘↓ | Ctrl+Alt+↑/↓ | Collapse / expand all projects |
| ⌥⌘← / ⌥⌘→ | Ctrl+Alt+← / → | Show lists (sidebar) / show tags panel |
| ⌥⌘J | Ctrl+Alt+J | Toggle Project View |
| ⌘J | Ctrl+J | Toggle Focus Filter |
| ⌘K | Ctrl+K | Show Scheduled |
| ⌘U | Ctrl+U | Show Paused |
| ⌘⇧K | Ctrl+Shift+K | Show Scheduled & Paused |
| ⌘O | Ctrl+O | Show Completed |
| ⌘F | Ctrl+F | Find |
| ⌥⌘F | Ctrl+Alt+F | Toggle search scope (this list / all) |
| ⌥⌘⇧F | Ctrl+Alt+Shift+F | Find options (smart search builder) |
| ⇧⌘O | Ctrl+Shift+O | Quick Open |
| ⌘S | — | Sync (out of scope) |
| ⌘P / ⇧⌘P | Ctrl+P / — | Print / Page setup (optional) |
| ⌘, | Ctrl+, | Preferences |
| ⌘W / ⌘Q | Ctrl+W / Ctrl+Q | Close / Quit |

Inside popovers [obs]: defer quick buttons ⌥1 Today, ⌥2 Tomorrow, ⌥3 next weekday, ⌥4 next week;
action picker ⌥1–⌥6 (call, message, email, drive, link, search).

**Global** (system-wide): Quick Entry hotkey, user-configurable (ShortcutRecorder). On Omarchy:
`bind = SUPER SHIFT, T, exec, yourapp --quick-entry` in `~/.config/hypr/bindings.conf`.
