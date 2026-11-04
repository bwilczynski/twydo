# A tour of the TUI on the synthetic demo vdir: completing a task, adding one with o,
# turning it into a project and adding its items with a, adding checklist items in a chain,
# the edit card with a changed due date, and the help overlay.
Output "docs/demo/tui.gif"
Set Width 2040
Set Height 1100
Source "docs/demo/common.tape"

Type "tdo"
Sleep 300ms
Enter
Sleep 2.5s

# All tasks, grouped by date.
Type "2"
Sleep 2s

# Find "Pay electricity bill" and complete it, then clear the search.
Type "/"
Type@100ms "electricity"
Sleep 600ms
Enter
Sleep 600ms
Type "x"
Sleep 2s
Type "/"
Escape
Sleep 600ms

# Add a task below the selected one.
Type "o"
Sleep 600ms
Type@100ms "Book the venue"
Sleep 1.2s
Enter
Sleep 1.5s

# Make it a project, then add two items: Enter saves one and opens the next row.
Type "cp"
Sleep 1.2s
Type "a"
Sleep 800ms
Type@100ms "Find a venue"
Sleep 600ms
Enter
Sleep 1s
Type@100ms "Send invites"
Enter
Sleep 800ms
Escape
Sleep 1.5s

# Add items to the existing packing checklist the same way.
Type "/"
Type@100ms "packing"
Sleep 600ms
Enter
Sleep 600ms
Type "a"
Sleep 700ms
Type@100ms "Sunscreen"
Enter
Sleep 800ms
Type@100ms "Hat"
Enter
Sleep 800ms
Escape
Sleep 1.5s

# Open the edit card on a weekly task and move its due date to next Wednesday.
Type "/"
Escape
Sleep 500ms
Type "/"
Type@100ms "water plants"
Sleep 600ms
Enter
Sleep 600ms
Type "i"
Sleep 1.2s
Tab
Tab
Tab
Sleep 500ms
Ctrl+U
Type@120ms "wed"
Sleep 2s
Enter
Sleep 2s

# Clear the search and show the help overlay with the new keys.
Type "/"
Escape
Sleep 500ms
Type "?"
Sleep 3s
Escape
Sleep 500ms
Type "q"
Sleep 500ms
