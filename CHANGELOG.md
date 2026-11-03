# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/bwilczynski/twydo/releases/tag/v0.1.0) - 2026-11-03

### Added

- *(tui)* offer to stop a running sync when a save is blocked
- *(tdo)* wait briefly for the vdir lock and stop a sync to save
- *(tui)* confirm before completing a parent with open items
- *(tui)* movement keys scroll the Details pane when it has focus
- *(tui)* the details pane lists a parent's tasks
- *(tui)* edit notes in $EDITOR
- *(tui)* edit a task in a form
- *(tui)* edit form state
- *(tui)* move a task to another list
- *(tui)* toggle done, due shortcuts, delete, undo and redo
- *(tdo)* a writer for TUI operations
- *(sync)* close a conflict by keeping the task as it is now
- *(tui)* sync status, :sync and conflict resolution
- *(tdo)* sync in the background after each write
- *(sync)* resolve conflicts by keeping either version
- *(sync)* handle emptied collections without losing tasks
- *(sync)* confirm journal entries and catch delete-versus-edit
- *(sync)* serialized sync runner with backoff and status
- *(sync)* conflict hook that keeps both versions
- *(store)* mark journal entries synced and prune them
- *(tdo)* tdo op commands and journal recovery at startup
- *(core)* write gate driven by the compatibility matrix
- *(core)* embedded matrix with iphone_baseline column; write mode config
- *(tui)* terminal loop with live reload
- *(tui)* three-pane rendering with ANSI theming
- *(tui)* app state and vim-style navigation
- compatibility matrix, tdo check and export-json

### Fixed

- *(sync)* honor stops before runner initialization finishes
- *(sync)* a pass begun before a stop never starts vdirsyncer
- *(sync)* address PR review of stopping a sync
- *(sync)* close review gaps in stopping a sync
- *(sync)* hold off new syncs until the stopping save is done
- *(tui)* open and show the stop question while a sync holds the vdir
- *(tdo)* check uses the built-in matrix unless --matrix is given
- *(tui)* dropping a pending x also clears its confirmation question
- *(tui)* use the singular for a family of one item
- *(tui)* the clock guard drops only keys that act on the selected task
- *(tui)* a key never lands on a task the clock moved under the cursor
- *(tui)* the details pane offers Enter only where it expands
- *(tui)* pending keys, date keys and reload errors around writes
- *(tui)* undo, redo and stale messages name the task
- *(tui)* the edit form trims titles and ignores a repeat of the same number
- *(tui)* the move picker scrolls to keep the selected list visible
- *(tui)* success messages clear after five seconds
- *(tui)* a write while a sync runs is refused instead of freezing the UI
- *(tui)* a form saves against the version it opened on
- *(tui)* pasted text fills a form field and never runs as keys
- *(tui)* overlays swallow keys that write
- *(tui)* address review of layout, theme and focus changes
- *(tui)* improve responsive layout, theme styling and pane focus
- *(sync)* a failed sync of a list drops its auto-force evidence
- *(sync)* an unreadable manifest never stops a sync
- *(sync)* check the task before resolving a conflict as already in place
- *(sync)* auto-force an emptied list only when the journal accounts for every task
- *(tui)* sync after a resolve only when it worked and after_edit is on
- *(sync)* let vdirsyncer hold its own vdir lock; pass the vdir to the hook
- *(sync)* keep deletions and settle late server changes in reconcile
- *(sync)* word the forced-list message accurately and only for automatic forces
- *(sync)* keep deletions behind held conflicts and record delete conflicts once
- *(store)* clearer errors for a missing vdir; correct moved-vdir docs
- *(tdo)* hold the reader lock while tdo op loads its snapshot
- *(store)* keep journal and lock per vdir
- readers load the vdir under the shared lock
- *(tui)* stop reload loop that reset the sidebar cursor
- *(tui)* show load errors, one sidebar order, per-row overdue red
- *(check)* report blocked open tasks and fail the gate on them
- gate fails on parse errors and unverified modifiers
