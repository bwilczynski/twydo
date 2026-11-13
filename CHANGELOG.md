# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1](https://github.com/bwilczynski/twydo/compare/v0.3.0...v0.3.1) - 2026-11-13

### Added

- *(cli)* name each list next to its id in tdo status
- *(tui)* show the inbox first in the focus lists
- *(tui)* toggle parents with Enter in every view

### Fixed

- *(tui)* keep the Enter chain's row after items that arrive late
- *(lists)* default the helper's interpreter to vdirsyncer's own

## [0.3.0](https://github.com/bwilczynski/twydo/compare/v0.2.2...v0.3.0) - 2026-11-12

### Added

- *(tdo)* guided setup with tdo config init
- *(tdo)* tdo config check
- *(config)* [**breaking**] own the CalDAV settings and generate the vdirsyncer config
- *(sync)* render the vdirsyncer config from [sync]
- *(daemon)* protocol generation 2

### Fixed

- *(tdo)* config check password hint depends on the failure
- *(lists)* keep config-path log out of the fake server state; sync schema description
- *(tdo)* config check emulates vdirsyncer's password reading and checks the state dir is writable
- *(tdo)* client no-config error points at tdo config init
- *(config)* reject empty init flags and blank password_command
- *(tdo)* config check verifies tdo is on PATH and keeps partial password output
- *(tdo)* write ca_file home-relative; stronger symlink test
- *(tdo)* config check keeps the tty, bounds pipe reads, skips by dependency
- *(lists)* refuse to resume a list operation recorded before the generated config
- *(lists)* recovery points the helper at the generated vdirsyncer config
- *(sync)* ensure the generated config after the shutdown hold; document test seam; inode test
- *(sync)* generate the vdirsyncer config only under the run lock

## [0.2.2](https://github.com/bwilczynski/twydo/compare/v0.2.1...v0.2.2) - 2026-11-12

### Added

- *(tdo)* delete lists from the sidebar
- delete lists from CalDAV after a verified backup
- enable best-effort list rename and recolor
- *(helper)* delete a list collection after revalidating its backup
- *(compat)* accept best-effort list metadata and deletion

### Fixed

- *(tdo)* close only the finished list's delete confirmation
- *(helper)* check for a vdirsyncer 404 without requiring vdirsyncer
- *(core)* finish interrupted, partial and unsent list deletions
- *(core)* record the retired copy of a deleted list in its operation
- *(helper)* reconcile an uncertain delete despite local edits
- always retire a list the server has deleted
- *(helper)* accept only 200, 202, 204 or 404 as a completed delete

## [0.2.1](https://github.com/bwilczynski/twydo/compare/v0.2.0...v0.2.1) - 2026-11-11

### Added

- *(compat)* verify discovery of phone-created lists ([#76](https://github.com/bwilczynski/twydo/pull/76))
- *(tdo)* add tdo sync --discover and :discover
- *(daemon)* discover lists at startup and on request
- *(sync)* discover lists created on other devices

## [0.2.0](https://github.com/bwilczynski/twydo/compare/v0.1.9...v0.2.0) - 2026-11-11

### Added

- *(compat)* [**breaking**] replace session gates with operation contracts
- *(compat)* define typed operation contracts and v2 validation

### Fixed

- *(compat)* name the refused write and its matrix entry in refusals

## [0.1.9](https://github.com/bwilczynski/twydo/compare/v0.1.8...v0.1.9) - 2026-11-11

### Added

- *(tdo)* warn while no service unit is installed
- *(tdo)* the daemon's service unit: install, status, start, stop and upgrade
- *(tdo)* service unit naming, rendering and manager commands
- *(tdo)* tdo status asks the daemon; no client reads the vdir
- *(daemon)* conflicts.show; tdo conflicts reads through the daemon
- *(daemon)* export; tdo export-json asks the daemon
- *(daemon)* check.report; tdo check and op check ask the daemon
- *(tdo)* list and recovery panels read through the daemon
- *(tdo)* the TUI runs on the daemon; remove the file and preference watchers
- *(tdo)* a link thread that feeds the TUI from the daemon
- *(tdo)* tdo sync and conflict resolution through the daemon; remove the batch waiter
- *(tdo)* tdo op talks to the daemon
- *(tdo)* connect to the daemon, starting it on demand
- *(daemon)* per-list sync state, lists.pending and an input.parse anchor
- *(daemon)* list and resolve conflicts with replayable resolutions
- *(daemon)* reap an orphaned vdirsyncer and stop cleanly during a sync
- *(daemon)* stop a running sync and switch automatic sync
- *(daemon)* run syncs as jobs on the vdir lane with job.get
- *(daemon)* watch the vdir and add the hidden tdo daemon command
- *(core)* record requests on journal entries and queue items
- manage lists and recovery from the TUI
- gate title and notes edits by preserved fields
- *(daemon)* run list create, update, archive and delete as jobs
- *(daemon)* reload config.toml live and parse typed dates and repeats
- *(daemon)* journal list, recover, revert and abandon
- *(daemon)* serve lifecycle methods on a private Unix socket
- *(daemon)* add wire types for tasks and snapshots
- preserve and export raw collection deletion backups
- manage CalDAV lists through a durable vdirsyncer adapter
- persist local list archiving
- *(daemon)* forget and recover lists as jobs and list their backups
- *(daemon)* schedule startup, periodic and batched syncs
- *(daemon)* let clients read enum values they do not know
- *(daemon)* queue writes during long jobs and drain them after
- *(daemon)* apply task writes with sessions, replay and based_on
- *(daemon)* publish snapshots, status and change events
- *(daemon)* generate the OpenRPC schema and guard breaking changes
- *(daemon)* define the JSON-RPC protocol types

### Fixed

- *(daemon)* answer panicking requests and drop clients that stop reading
- *(daemon)* back off on accept errors, exit on a second signal, report unreadable queue items
- *(daemon)* keep the outcome-event order and finish drained items of a failed sync
- *(daemon)* replay creates and cancelled undos exactly, index requests
- recover interrupted list metadata writes
- *(daemon)* show queued edits as pending while a sync holds the vdir

## [0.1.8](https://github.com/bwilczynski/twydo/compare/v0.1.7...v0.1.8) - 2026-11-08

### Fixed

- *(tui)* strike through completed task titles
- *(tui)* keep completion cursor out of Done

## [0.1.7](https://github.com/bwilczynski/twydo/compare/v0.1.6...v0.1.7) - 2026-11-08

### Added

- add gated item relocation and structure commands
- gate recurring checklist scheduling by write scope
- gate task and family moves by relocation effects
- reopen detached history by lifecycle effects
- admit deletion by exact resource scope

### Fixed

- avoid repeated collection scans in permission reports
- *(cli)* keep queued refusal details out of permission reports
- *(cli)* align task checks with queued operation permissions
- *(cli)* separate data health from operation permissions

## [0.1.6](https://github.com/bwilczynski/twydo/compare/v0.1.5...v0.1.6) - 2026-11-07

### Added

- *(tui)* reorder checklist and project items
- enable verified checklist and project item deletion

## [0.1.5](https://github.com/bwilczynski/twydo/compare/v0.1.4...v0.1.5) - 2026-11-06

### Added

- *(tui)* show sync activity in the status line
- *(op)* queue writes during a sync; --wait for the outcome
- *(tui)* queue saves during a sync instead of asking to stop it
- *(sync)* the runner drains the write queue
- *(store)* undo and redo of queued edits
- *(views)* show queued edits over the loaded files
- *(tui)* choose lists by name when forgetting
- batched syncs after edits and an automatic-sync switch
- *(sync)* batch waiter process (tdo sync --background --when-due)
- *(sync)* batching rule and automatic-sync switch
- *(config)* add batch_size and max_wait_secs; retire after_edit
- *(tui)* reload when any sync ends
- limit list Done sections to the last 30 days
- show future and completed work in lists
- *(store)* one write path that queues while the vdir is busy
- *(store)* on-disk write queue
- *(sync)* record a sync-ended event for every run

### Fixed

- *(op)* say once that --wait is waiting for a sync; document queued op in sync.md
- *(sync)* preserve missing-list guidance during rediscovery
- *(tui)* fit forget confirmation to its wrapped text
- *(sync)* turn sync on even when the edits cannot be read
- *(sync)* keep edits counted after an orphan stop
- *(sync)* wait for a pause before a size-triggered sync
- *(config)* keep after_edit = false suppressing syncs until batching lands
- preserve selection when reopening detached children
- *(sync)* capture manifest before draining queued edits
- *(store)* keep recoverable queued writes pending after I/O errors
- *(store)* preserve commit order when draining queued edits
- *(sync)* no rerun after a stopped pass; the queue never fails a sync
- *(store)* undo skips a queued step a drain refused
- *(sync)* age drained edits from when they were queued
- *(store)* keep a submitted item queued when the recheck drain fails
- *(sync)* recheck batch deadlines while the waiter sleeps

### Performance

- *(views)* list the journal once per overlay

## [0.1.4](https://github.com/bwilczynski/twydo/compare/v0.1.3...v0.1.4) - 2026-11-05

### Added

- confirm deleted lists and recover preserved tasks

### Fixed

- keep the sync message visible when the reload after it fails
- refresh task lists after manual sync
- sync backup directories before retiring lists
- preserve retired lists and recover their local tasks safely

## [0.1.3](https://github.com/bwilczynski/twydo/compare/v0.1.2...v0.1.3) - 2026-11-05

### Added

- *(compat)* unlock scoped repeating checklist creation and item deletion
- *(compat)* verify scoped from-completion editing
- *(compat)* verify until completion and occurrence reopening
- *(compat)* verify monthly recurring checklist workflows
- *(compat)* verify plain daily and yearly repeat changes
- *(core)* prepare scoped checklist writes with durable family checks

### Fixed

- use the same family item order in details
- keep the next item row at the end of its family
- *(compat)* restrict completion repeat changes to captured transitions
- append family items with guarded ordering positions
- preserve checklist and project item display order
- *(store)* retain checklist family guards through journal replay
- *(core)* assign new checklist history parent identities

## [0.1.2](https://github.com/bwilczynski/twydo/compare/v0.1.1...v0.1.2) - 2026-11-04

### Added

- *(compat)* verify completing every repeat kind and editing daily and weekly tasks
- *(compat)* allow weekly and monthly repeat changes in verified mode
- *(compat)* allow type changes in verified mode
- *(tui)* change a task's type with ct, cp and cc
- *(tui)* add tasks and items inline with o, O and a
- *(tui)* edit tasks in an in-place card with list, type and repeat
- *(cli)* add items, kinds and repeats with tdo op
- *(core)* gate new items, repeats and conversions; grouped save
- *(core)* create projects, checklists, items and repeating tasks
- *(core)* complete every repeat kind Twydo can create
- *(core)* change a task's type between task, checklist and project
- *(core)* set and remove repeat rules as 2Do writes them
- *(core)* parse typed repeat rules

### Fixed

- *(compat)* keep completing until series unverified
- *(gate)* say 2Do cannot show the repeat kinds it misreads, and follow the verified matrix
- choosing the type a task already declares writes nothing
- *(tui)* say that emptying a repeating task's repeat removes it
- *(gate)* setting, removing or reshaping a repeat needs the repeat rows
- *(core)* keep the original repeat count when a repeat edit leaves the ending unchanged
- *(repeatinput)* split a short period on a character, not a byte
- *(gate)* refuse an interval on a repeat on set weekdays
- *(store)* hold the parent when creating an item
- *(store)* rescan a task's items before changing its type
- *(gate)* refuse changing the interval of an Nth-weekday repeat
- *(gate)* say why reopening an ended series is refused before the matrix rows
- *(gate)* refuse completing a recurring project that ends or repeats from completion
- *(core)* write 2Do's own code 6 for a last-weekday repeat
- *(store)* treat ending a series in place as a repeat completion
- *(core)* complete every repeat kind as the phone does
- *(gate)* check a reshaped repeat against its repeat row, and tidy the matrix and docs
- *(core)* refuse writing a repeat without 2Do's JSON metadata
- *(core)* close gate holes in repeats without a due, saves that move, and closed parents
- *(core)* refuse creating parents, items and repeats until verified
- *(core)* keep an existing series id when setting a repeat

## [0.1.1](https://github.com/bwilczynski/twydo/compare/v0.1.0...v0.1.1) - 2026-11-03

### Fixed

- *(tui)* wait for saves to pause before the after-edit sync
- *(tui)* keep the selection when a repeat completion is refused
- *(tui)* keep the cursor on its row when completing a repeating task

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
