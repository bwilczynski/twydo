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
- Changes remain uncommitted for the user’s diff review. No NAS, phone, production account or task data was accessed.

## Approved review follow-ups

The user requested the original work be committed first and each follow-up be committed separately.

1. Advertise bare `:forget-list` in the status line, errors panel and CLI sync guidance. Retain the ID-based shortcut. Test the TUI guidance and CLI report.
2. Mark rows with a recorded per-list Not Found sync error in the Forget list picker. Preserve other rows and actions; label the report without inferring deletion. Test missing, healthy and other-error rows, including long labels.
3. Size retirement confirmation to its wrapped text and give it a matching title. Retain the larger recovery preview. Test compact placement, wrapped text on a narrow terminal, and recovery confirmation preview.

Verification: targeted tests per commit, then full lint, tests and documentation links.
