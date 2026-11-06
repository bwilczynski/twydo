# Issue 46: Forget-list chooser implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this single task with test-first verification.

**Goal:** Running `:forget-list` opens a list chooser so the user never needs to type a collection ID.

**Architecture:** Reuse the TUI list picker with an explicit retirement purpose. Display list names and select by collection ID internally. Show IDs alongside names only when needed to distinguish duplicate display names; unnamed collections fall back to their ID. Populate the chooser from all loaded collections, including lists excluded from the sidebar. Enter calls the existing retirement preview and confirmation; only `y` emits the existing retirement outcome. Preserve named-command support and the existing recovery browser/destination chooser.

**Tech Stack:** Rust, ratatui, existing App command tests and synthetic vdir fixtures.

**Spec:** [Issue #46](https://github.com/bwilczynski/twydo/issues/46), extending the [list retirement design](../specs/2026-11-05-issue-42-deleted-list-recovery-design.md).

## Constraints and review focus

- No NAS, phone, production data, compatibility verdict or core write-path changes.
- Cancellation must clear picker intent and leave files untouched.
- Duplicate names must be distinguishable in the chooser and select by collection ID.
- Empty and hidden lists must remain selectable; an empty collection set needs a clear message.
- Move, edit-card list selection and recovery destination selection must retain their behavior.

## Task 1: Chooser and documentation

Files: `crates/tdo/src/app.rs`, `crates/tdo/src/app/recovery.rs`, `crates/tdo/src/ui/picker.rs`, `crates/tdo/src/ui/mod.rs`, `crates/tdo/tests/recovery.rs`, `docs/tui.md`.

- [x] Add failing command/render tests: bare command opens the correctly titled chooser; keyboard selection leads to confirmation for the chosen ID; Esc and declined confirmation preserve data; duplicate names show IDs; empty and sidebar-hidden collections are handled.
- [x] Run `cargo test -p tdo --test recovery` and confirm new cases fail for missing chooser behavior.
- [x] Add picker purpose/state, retire-list selection handling, and purpose-specific title/hints. Reuse existing preview/confirmation and preserve argument-based commands.
- [x] Update command help and as-is TUI documentation for the optional argument.
- [x] Run targeted tests, `mise run lint`, `mise run test`, and `git diff --check`.
- [x] Review the final diff and offer it for user review before committing.

## Execution record

Approved in the issue #46 session, including name-first list display. Implemented in the Herdr worktree on `fix/46-forget-list-picker`.

- Added five synthetic chooser tests. The original four failed on missing bare-command behavior before implementation; the long-name case failed on hidden disambiguation IDs before its fix.
- Existing keyboard tests now assert the explicit picker action in place of the former optional UID. The existing move lifecycle test caught early dismissal; preserving the picker until `write_finished` restored that behavior.
- Separate code review checked cancellation, ID-based selection, empty/hidden lists and existing picker actions. Both findings were fixed and covered by failing-then-passing tests. The long duplicate-name finding was treated as required disambiguation, rather than deferred polish.
- Recovery and keyboard tests pass; final workspace lint, complete tests and documentation links are checked before handoff.
- At the initial handoff, changes were left uncommitted for the user’s diff review. No NAS, phone, production account or task data was accessed.

## Approved review follow-ups

The user requested the original work be committed first and each follow-up be committed separately.

1. Advertise bare `:forget-list` in the status line, errors panel and CLI sync guidance. Retain the ID-based shortcut. Test the TUI guidance and CLI report.
2. Mark rows with a recorded per-list Not Found sync error in the Forget list picker. Preserve other rows and actions; label the report without inferring deletion. Test missing, healthy and other-error rows, including long labels.
3. Size retirement confirmation to its wrapped text and give it a matching title. Retain the larger recovery preview. Test compact placement, wrapped text on a narrow terminal, and recovery confirmation preview.

Verification: targeted tests per commit, then full lint, tests and documentation links.

### Follow-up results

All three follow-ups are implemented in separate commits, after the initial plan and chooser commits. Error guidance covers the status line, errors panel and CLI report; chooser markers use existing per-list status, including the trailing-semicolon error form. Confirmation height uses the wrapped text’s inner width, including borders correctly; width-sweep regressions cover 30–120 columns and recovery still displays its preview.

A separate review found the border-width calculation issue; its failing width-sweep test passed after correction. No remaining review findings. Final verification: `mise run lint`, `mise run test` (912 Rust tests and 49 Python tests, plus documentation links), and `git diff --check` passed. No device or production access.

## Multiple missing lists follow-up

User-reported regression: retirement triggers discovery, and failed discovery replaces previously reported per-list errors with an empty map. Preserve unresolved Not Found reports only for still-active local lists when discovery fails. Recognize remote missing-collection discovery diagnostics so a newly reported missing list is also visible; exclude local-storage warnings. Keep this reporting separate from reconciliation and force-delete decisions. A subsequent ordinary sync replaces the retained reports.

Tests: synthetic runner with multiple prior missing lists and failed discovery/retries/success; real filesystem-pair discovery with two remotely removed lists and a surviving list; no recreation or unconfirmed retirement. Update sync docs and commit this bug fix separately after lint/full tests.

Multiple-list results: all three new regressions failed before the fix and passed afterward. The real vdirsyncer filesystem test confirms that the remaining missing list is reported after the first retirement, its local task stays intact until separately confirmed, neither removed server list is recreated, and syncing resumes with the surviving list. Separate review found no remaining issues. `mise run lint`, `mise run test` (915 Rust tests, 49 Python tests, documentation links), and `git diff --check` passed.

## Generic discovery failure follow-up

The user still sees the exact generic discovery failure with no remaining-list guidance. The prior fix assumed available per-list reports. It also left the prepended generic error as the first unattributed line, hiding vdirsyncer's actual diagnostic.

Add failing tests for the exact saved generic error with an empty collection map, and a failed-discovery runner with no collection diagnostics. Show `:forget-list` guidance in the status/error views even for that saved error; retain actual discovery stderr in the reported failure so unrecognized diagnostic formats remain visible. Do not mark unrecognized lists as known missing or advise forgetting lists the user did not delete. Run full verification and commit separately.

Generic-error results: the exact legacy-message TUI regression and the runner guidance assertion failed before the correction. Both pass now, and the runner retains the actual diagnostic in persisted status. Separate review found no remaining issues. Full verification passed: `mise run lint`, `mise run test` (916 Rust tests, 49 Python tests, documentation links), and `git diff --check`.
