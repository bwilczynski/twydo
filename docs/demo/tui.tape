# A tour of the TUI on the synthetic demo vdir: views, completing a task, a checklist,
# the edit form with natural dates, a list, and the help overlay.
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

# Complete "Pay electricity bill" (fourth row).
Type@400ms "jjj"
Sleep 800ms
Type "x"
Sleep 2s

# Expand the packing checklist (the row above the last one).
Type "G"
Sleep 400ms
Type "k"
Sleep 600ms
Enter
Sleep 2s

# Edit "Call the dentist": move it to Friday.
Type "gg"
Type "j"
Sleep 600ms
Type "i"
Sleep 1.2s
Tab
Sleep 500ms
Ctrl+U
Type@120ms "fri 10am"
Sleep 1.8s
Enter
Sleep 2s

# Open the Bills list from the sidebar.
Type "h"
Sleep 400ms
Type@300ms "jjj"
Sleep 400ms
Enter
Sleep 2.5s

# Help overlay.
Type "?"
Sleep 3s
Escape
Sleep 500ms
Type "q"
Sleep 500ms
