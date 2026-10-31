# TUI layout (chosen design)

The user compared three options (terminal app, Tauri, Quickshell + SwiftUI) using HTML mockups
and chose the **terminal app**. This is the chosen layout, redrawn with **example data**: titles and
list names are placeholders, not the user's real tasks. Behaviour and keys are defined in the spec
(`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` §7–8). This file only shows the shape.

## Main screen (Today view)

```
╭ Views ───────────╮╭ Today · Saturday 31 Oct ────────────────── ↓ due · ☼ ╮
│ ◷ Today      1│3 ││ OVERDUE                                            1 │
│   All        3│9 ││ ▌↺ Internet bill                monthly   7 days ago │
│   Scheduled    2 ││                                                      │
│   Done           ││ TODAY                                              2 │
├ Lists ───────────┤│ ▌↺ Monthly tax payment          monthly   Today      │
│ ▌ Inbox          ││ ▌[5] Monthly bookkeeping        project   Today      │
│ ▌ Bills      1│6 ││                                                      │
│ ▌ Business     4 ││ TOMORROW                                           2 │
│ ▌ Personal       ││ ▌↺ Social insurance             monthly   Tomorrow   │
│ ▌ Work           ││ ▌↺ Sports club fee              monthly   Tomorrow   │
│ ▌ Shopping       ││                                                      │
│ ▌ Health         ││ NEXT 7 DAYS                                        2 │
├ Smart lists ─────┤│ ▌↺ Send invoice                 monthly   Thu        │
│ ▌ Next 3 Days    ││ ▌↺ Phone bill                   monthly   Sat        │
╰──────────────────╯╰──────────────────────────────────────────────────────╯
 NORMAL  j/k move x done t today ] +1d o new / search : cmd  Today·3/7
```

On wide terminals a **details pane** sits to the right of the task list (collapsible on narrow ones):

```
╭ Details ───────────────────────╮
│ Monthly tax payment            │
│                                │
│ List    Business               │
│ Due     Today                  │
│ Repeat  Monthly on the 25th    │
│ Alert   09:00                  │
│ Start   1 day before           │
│ ── Notes ───────────────────── │
│ (markdown)                     │
╰────────────────────────────────╯
```

## Elements

| Element | Meaning |
|---|---|
| `▌` (left of each row and list) | the list's colour (true colour from CalDAV `calendar-color`) |
| `1│3` counts | due-or-overdue │ total (definition in spec §8) |
| `↺` | repeating task · `○` one-time task · `[5]` parent with 5 items |
| `OVERDUE` header and rows | theme red (ANSI); all other chrome uses the terminal's 16 ANSI colours |
| `↓ due · ☼` | current sort (due date, descending/ascending) and focus filter indicator |
| Status line | vim mode (`NORMAL`/`INSERT`/`COMMAND`), key hints, view position |

## Natural-language add (`o`)

```
╭─ New task ─────────────────────────────────────────────────────────╮
│ Pay kindergarten fee fri 9am every month list(Bills)▏              │
│ → Bills · due Fri 6 Nov 09:00 · repeats monthly                    │
╰────────────────────────────────────────────────────────────────────╯
 INSERT  Enter save  Esc cancel  Tab next field
```

## Theming

All UI chrome uses the terminal's ANSI palette, so Omarchy theme switches re-colour the app with
no code. On macOS it follows whatever theme the terminal (e.g. Ghostty) uses.
2Do's own colours, for reference only: `docs/reference/2do/03-look-and-feel.md` §7.
