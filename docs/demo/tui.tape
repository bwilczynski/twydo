# A tour of the TUI on the synthetic demo vdir: adding a task in the Inbox, moving it to
# another list and turning it into a checklist with items, completing a task due today,
# editing a note from All, and a manual sync. render.sh supplies a stand-in sync tool.
Output "docs/demo/tui.gif"
Set Width 2040
Set Height 1100
Source "docs/demo/common.tape"

# The daemon's startup sync runs off-screen, so the only sync on camera is the manual one.
Hide
Type "tdo sync >/dev/null && clear"
Enter
Sleep 4s
Show

# 1. Start the TUI.
Type "tdo"
Sleep 300ms
Enter
Sleep 2.5s

# 2. The Inbox: add a task.
Type "0"
Sleep 1.2s
Type "o"
Sleep 600ms
Type@100ms "Plan the reunion"
Sleep 800ms
Enter
Sleep 1.5s

# 3. Move it to Personal, follow it there and make it a checklist with three items.
Type "m"
Sleep 800ms
Type@100ms "Personal"
Sleep 800ms
Enter
Sleep 1.5s
Type ":"
Type@80ms "list Personal"
Sleep 500ms
Enter
Sleep 1.5s
# The moved task is the last row: go there without filtering, so its items stay in view.
Type "G"
Sleep 1s
Type "cc"
Sleep 1.5s
Type "a"
Sleep 700ms
Type@100ms "Book a room"
Enter
Sleep 800ms
Type@100ms "Send invites"
Enter
Sleep 800ms
Type@100ms "Order a cake"
Enter
Sleep 800ms
Escape
Sleep 2.5s

# 4. What's due today: complete a task.
Type "1"
Sleep 2s
Type "/"
Type@100ms "dentist"
Sleep 600ms
Enter
Sleep 800ms
Type "x"
Sleep 2s
Type "/"
Escape
Sleep 1.5s

# 5. All tasks: find the water bill and add a line to its note in the editor.
Type "2"
Sleep 1.5s
Type "/"
Type@100ms "water bill"
Sleep 600ms
Enter
Sleep 800ms
Type "i"
Sleep 1.2s
Tab 6
Sleep 800ms
Enter
Sleep 1.5s
Type "i"
Type@80ms "Pay online this month"
Sleep 800ms
Escape
Type ":wq"
Sleep 400ms
Enter
Sleep 1.5s
Ctrl+S
Sleep 2.5s
Type "/"
Escape
Sleep 1s

# 6. Sync now: the spinner, then "synced just now".
Type ":"
Type@80ms "sync"
Sleep 500ms
Enter
Sleep 4s
Type "q"
Sleep 500ms
Hide
Type "tdo daemon stop"
Enter
Sleep 1s
