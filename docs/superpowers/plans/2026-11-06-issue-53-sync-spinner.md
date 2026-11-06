# Issue #53: sync spinner implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan in the current session. Track progress with the checkboxes below.

**Goal:** Show an animated spinner with `Syncing…` during sync, then swap it directly for the normal last-sync age.

**Architecture:** Keep transient sync activity separate from ordinary messages, so typing or saving cannot erase the indicator. The terminal loop observes manual sync workers, the existing live running record, and the existing sync-ended marker. Render activity in the existing sync segment, retaining pending counts, conflicts, automatic-sync-off, and error diagnostics.

**Tech Stack:** Rust, ratatui, existing terminal poll loop and sync state files; no dependencies.

**Spec:** [Issue #53](https://github.com/bwilczynski/twydo/issues/53).

## Global constraints

- Use terminal ANSI theme colours; no task-data writes or device tests.
- Show activity and last-sync age in the same slot: activity while running, age when idle.
- Builder's call: animate on the existing 250 ms loop using monotonic time. The user's follow-up removes the transient completion phase.
- Failure, conflicts, and emptied-list outcomes must never appear as successful completion.
- Update `docs/tui.md` and run the documentation link checker.

## Review focus

- Background runs and runs started outside this TUI get the same indicator.
- A run that finishes between polls is noticed through the ended marker.
- Finishing a sync restores the age immediately, without a transient completion label.
- Stale running records must not leave an endless spinner.
- Editing and command prompts retain sync activity and diagnostics.

### Task 1: Sync activity, rendering, and terminal integration

**Files:** `crates/tdo/src/app/sync_activity.rs`, `crates/tdo/src/app.rs`, `crates/tdo/src/term.rs`, `crates/tdo/src/ui/status.rs`, `crates/tdo/tests/ui_render.rs`, terminal-loop unit tests, `docs/tui.md`.

**Interfaces:** App-owned activity state with explicit monotonic-time updates; terminal polling supplies running/ended observations; status rendering reads the current activity label.

- [x] Add failing tests for animation, replacement of the last-sync age, persistent errors/counts, and observed background completion without watcher events.
- [x] Run focused tests and confirm the missing activity behaviour fails.
- [x] Implement the activity state and renderer. Poll live running state and sync-ended marker without reloading the entire summary on each animation frame. Manual workers show activity immediately. Ending a run restores the persisted status instead of adding a second message.
- [x] Run focused tests, workspace tests, formatting/lint checks, and `python3 tools/check_links.py`.
- [x] Inspect the complete diff for scope and synthetic data only, then offer the completed diff for review before committing.

## Review record

The independent review found two manual-worker edge cases: existing unresolved conflicts
could be overridden by the child result, and warnings could hide successful completion.
The subsequent UI refinement removes the transient success indication and its outcome
tracking; the existing persisted sync status supplies the result. Regression tests retain
warning and error diagnostics. The ended watcher is initialized before the startup child is spawned, so a
fast startup sync cannot be mistaken for an old event.

Initial validation: 1,004 Rust tests and 49 Python tests passed; Clippy with warnings denied,
formatting, diff whitespace, and documentation links passed. Completed diff review was
offered; the user requested a commit.

## Follow-up: one status slot

The user observed that `Syncing… · synced just now` was misleading and requested a direct
swap from activity to the usual last-sync age. The renderer now suppresses the old age while
a run is active; the spinner stops immediately when the run ends. The transient completion
timer and worker outcome classification are removed. Existing errors and pending counts
remain visible. A rendering regression test reproduced the duplicate text before the fix
and now checks both `synced just now` and its progression to `synced 1m ago`.

Follow-up validation: 1,002 Rust tests passed; Clippy with warnings denied, formatting,
documentation links, and diff whitespace checks passed. The independent follow-up review
found no important issues.
