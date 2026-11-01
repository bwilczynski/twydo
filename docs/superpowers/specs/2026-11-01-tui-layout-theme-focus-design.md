# TUI layout, theme, and focus polish

Status: approved by user. Date: 2026-11-01.

## Intent and scope

Make list names readable on wide terminals, make the terminal theme visible in the
UI styling, and distinguish the selected task from the pane receiving keyboard input.
This is a presentation change to the existing Rust/Ratatui TUI. Task storage, sync,
and writable operations are outside its scope.

## Approaches

The user selected a responsive sidebar, terminal palette styling, and a subtle marker
on the selected task when another pane has focus. Removing every task selection cue
outside the task pane was considered; retaining a marker makes the Details context
clear. A fixed wider sidebar would still waste space on narrow terminals and would
not adapt to wide ones.

## Layout

Proposed sizing: from 60 columns, sidebar width is 20% of terminal width, clamped
to 24–40 columns. Below 60 columns, retain the current one-third allocation.
Examples: 80 → 24; 120 → 24; 160 → 32; 200 → 40 columns.
Keep the existing 36-column Details pane and its 110-column visibility threshold.
Reserve a cell for a task selection marker, accounting for it in title truncation.
Maintain aligned counts and dates, Unicode display widths, and safe tiny layouts.

## Theme styling

The current renderer already sends ANSI palette indices for chrome; the local
Ghostty configuration imports the active Omarchy palette. There is no evidence of
hard-coded RGB chrome. The visible weaknesses are sparse accent styling and the
large reverse-video task selection, which swaps foreground/background colors.

Use shared style roles for the terminal defaults, blue accent, muted text, active
selection, overdue red, and warning yellow. Use the accent consistently for active
pane borders/titles, sidebar headings, help keys, and the mode badge. Give the active
row a palette-based background rather than reverse video, preserving overdue text
and the list-color bar. All chrome stays within the terminal defaults and 16 ANSI
colors; CalDAV list-color bars remain the sole RGB exception. Do not import theme
files, copy their RGB values, or change desktop or terminal configuration.

## Focus behavior and issue #2

The task renderer currently applies REVERSED whenever the row equals the cursor,
regardless of pane focus. Consequently Details → Tasks can look like no change.
Only apply the active row background while Tasks has focus. When Sidebar or Details
has focus, retain a small muted marker beside the selected task with no row fill.
Give the active pane an accent border and title; show the focused pane name in the
status line when space permits. Use the same active selection styling in Sidebar.
Keep h/l and Tab navigation semantics. Ensure focus cannot remain visually invisible
in a collapsed Details pane when the terminal is narrow or resized.

## Validation

Use synthetic fixtures exclusively. Verify sidebar growth and long Unicode list
names at representative widths, full rendering on tiny terminals, and task selection
styling through recorded l/h/Tab transitions. Verify that overdue red and list bars
survive selection, inactive task rows lose active styling, and chrome uses no RGB
or palette indices above 15. Run tdo tests, workspace tests, formatting, and Clippy.
Visually inspect a synthetic TUI in the current terminal theme before completion.

## Implementation choices

The sizing formula, blue accent, palette-based active background, and status focus
label are implementation proposals. Keep existing dependencies; adjust the renderer
and focus visibility handling only where required. No production data is needed.
