# Capability gating 7: title and notes editing — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline, task by task. Track the checkboxes and record verification results here.

**Goal:** Admit title/notes edits independently of scheduling evidence, including the recurring-checklist shapes currently refused by the captured monthly/open-only boundary.

**Architecture:** Add a dedicated `resource-text` / `edit` capability before the legacy schedule/case checks. Reuse the existing field writer and keep missing-row fallback, structural validation, mixed-save contracts and whole-file concurrency checks. Do not widen broad operation or recurrence verdicts.

**Tech stack:** Existing Rust workspace, compatibility matrix, preserving iCalendar writer, journal, queue and synthetic fixture tests; no dependencies.

**Spec:** [Approved incremental capability gating design](../specs/2026-11-07-capability-gating-design.md), especially “Following increments”; [main spec](../specs/2026-10-31-terminal-tasks-design.md) §6 and §9.

## Global constraints

- Implement this increment locally and use synthetic inputs for runtime verification.
- Only local synthetic fixtures and temporary state/vdirs. No production data, installed-binary update, config changes, NAS, phone, sync, publication or push.
- Change only SUMMARY, DESCRIPTION, DTSTAMP and LAST-MODIFIED. Preserve all metadata, UID/RUID, parent links, dates, recurrence, alarms, ranks, lifecycle and other files exactly.
- Reuse representative phone text-edit evidence from Plan 4 and C29. Label broader schedule/status support as preservation generalization, not additional device observations.
- Unknown or ambiguous identities, unreadable required metadata, unsafe relationships, nesting, conflicts, stale bytes and incomplete recovery remain refused. Unknown unrelated content stays intact.
- A title/notes capability never authorizes date, repeat, interval, kind, lifecycle, relocation or creation changes. Mixed saves must independently pass their other contracts and remain atomic.
- User subsequently authorized focused commits, separating documentation from implementation. No attribution in commits or PRs. Offer a final terminal review using the committed range.

## Review focus

1. Completed/recurring parent and item text edits must preserve completion and every scheduling field.
2. Missing/unverified custom capability rows must not silently gain shipping permissions.
3. An empty changes vector must not grant arbitrary edits; blank titles must still fail without writes.
4. Text plus a refused schedule/type change must refuse the whole save and retain card input.
5. Queue drain, inverse and recovery must retain relevant identity/parent dependencies rather than overwrite remote changes.

## Task 1: Dedicated text contract and exact field preservation

**Files:** create `crates/twydo-core/src/gate/text_edit.rs`; modify `crates/twydo-core/src/gate.rs` and `compat/matrix.toml`; tests in existing `crates/twydo-core/tests/gate.rs` and `edit_fields.rs`; add evidence audit `docs/reference/phase0/text-edit-capability.md`.

**Interfaces:** `text_edit::check(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize, changes: &[Change], checker: &Checker<'_>) -> Option<Result<(), Blocked>>`. Return `None` when no dedicated row exists or the operation is not a nonempty title/notes-only edit. Otherwise validate the exact resource/flat-family structure and call `operation_row(matrix, mode, "resource-text", "edit")`.

- [x] Audit `apply_changes`, `set_summary`, `set_description` and `touch` against Plan 4/C29 text captures. Record direct evidence, preservation generalization, exact write set and excluded structures.
- [x] Add `text_capability_*` tests over existing recurring-checklist captures: open/completed parent and items, inherited/undated items, changed recurrence intervals, date times, starts, alarms, ends and completion anchors. Add standalone/project/from-completion controls. Assert title and notes are separately admitted regardless of unrelated schedule execution/edit verdicts.
- [x] Add negative tests for duplicate identities/properties, unreadable or duplicate metadata, broken/disagreeing links, cross-list/nested families and unknown task kinds. Preserve structural safeguards without requiring unrelated recurrence execution.
- [x] Test absent capability fallback, unverified capability refusal in verified mode, test-mode evidence semantics, blank title refusal at the writer/store and non-text edits retaining existing restrictions. Adapt older broad-row tests to explicitly exercise legacy matrices when needed.
- [x] Run `cargo test -p twydo-core --test gate --test edit_fields text_capability` and observe the intended admission assertion fail before adding the capability.
- [x] Implement the helper and early text-only dispatch after target lookup; add only the narrow matrix row with precise capture citations. Existing writer should need no change.
- [x] Prove exact preservation by comparing serialized resources after masking only the four allowed properties; keep complete metadata and VALARM bytes, parent/sibling/history files and identities unchanged. Run the focused tests and confirm they pass.

## Task 2: Mixed saves, queue, inverses and user-facing reporting

**Files:** tests in existing `crates/twydo-core/tests/store.rs`, `store_family.rs`, `store_queue.rs`, `journal.rs`, `crates/tdo/tests/cli.rs`, `cli_op.rs` and `write.rs`; change store code only if a new test exposes a dependency gap.

**Interfaces:** existing `Op::Edit` / `Op::Save`, `Store`, `Writer` and permission probes; no new CLI syntax or UI keys. Task 1's real gate supplies title/notes decisions.

- [x] Add a completed recurring-checklist text save with one journal entry; undo/redo restore exact originals while parent/items/history keep dates and lifecycle. Assert unrelated files stay byte-identical.
- [x] Add a refused mixed text/date, text/interval and text/type save on an unsupported shape: no partial write, no journal entry, card input retained. Verify an already-supported mixed due/text save remains allowed through its existing per-field contracts.
- [x] Queue a permitted text edit during a held vdir lock, drain once and compare the final bytes with immediate execution. Assert stale target bytes refuse without overwriting remote changes.
- [x] Exercise parent/identity changes between planning, queue drain and inverse/recovery where the new structural decision depends on them. Persist only dependencies the text contract actually needs; do not weaken whole-file checks.
- [x] Add CLI and real edit-card coverage for previously refused synthetic shapes; assert `tdo check` and `tdo op check` agree with the actual writer gate. Keep report output private by default.
- [x] Run affected core and CLI/TUI targets; observe any new workflow assertion fail before fixing its code, then confirm all affected targets pass.

## Task 3: Documentation, full validation and review

**Files:** `docs/compatibility.md`, `docs/tasks.md`, `docs/tui.md`, evidence audit and this plan.

- [x] Document the separate text contract and distinguish it from creation/conversion/scheduling gates. Explain direct versus generalized evidence and retained exclusions; update obsolete blanket title/notes restrictions.
- [x] Run `cargo test --workspace`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `git diff --check`. Use an existing disk-backed target directory; do not build in /tmp.
- [x] Record commands/results, enabled scope, exclusions and any implementation rulings below. Review the complete diff against the exact four-property write contract.
- [x] Offer final terminal diff review unless a live viewer is already running. Use a committed-range patch after the separately authorized focused commits.

## Execution ledger

- Planning audit: all six observed title/notes refusals use the recurring-checklist captured-shape gate; no task contents were inspected or copied. Existing text writer changes only SUMMARY/DESCRIPTION and normal stamps; metadata remains untouched.
- Existing design already approves text-capability migration and evidence reuse. This plan implements that decision rather than proposing a new permission policy.
- User selected implementation and subsequently authorized focused commits. Proceed inline; no additional design decision is needed for this already-approved capability policy.
- RED: completed checklist text admission failed against broad monthly edit evidence. GREEN: the dedicated text row now authorizes only nonempty SUMMARY/DESCRIPTION changes and normal stamps.
- RED: parent text undo ignored remote sibling changes; queued item text ignored a changed sibling. GREEN: durable family membership and byte dependencies now protect text edits through queueing, recovery and inverses.
- Ruling: mixed saves retain existing scheduling/type paths rather than globally splitting arbitrary field batches; supported checklist due/text composition uses the independent text contract, while existing unsupported scheduling combinations stay refused.
- Review: an independent reviewer identified member internal-ID collisions, JSON-only hidden descendants and missing durable member-alias dependencies. Added failing regressions, then per-member cached identity/reference checks and a union of held-member aliases in the existing membership guard. No journal schema change or additional rescan is needed.
- Ruling: retain the compatibility matrix's existing mode rules and missing-row fallback; update historical tests that asserted interval-2/weekly text refusal to expect text admission while still refusing unverified schedule edits.
- Implementation commit: `90cdec6` (`feat: gate title and notes edits by preserved fields`). Documentation is committed separately.
- Final independent review confirmed all three identity/dependency findings addressed; no remaining concrete issues.
- Validation: `cargo test --workspace --no-fail-fast` passed 1,155 Rust tests across 46 targets, zero failures/ignored; `python3 -m unittest discover -s tools/tests -t .` passed 49 tests; `python3 tools/check_links.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, `git diff --check` and staged whitespace checks passed.
- Enabled: nonempty title/notes-only operations on recognized standalone resources and safe flat task families, independent of held scheduling/lifecycle. Excluded: malformed identities/metadata/fields, unsafe relationships and all unverified non-text operations. No production permission recount or installed-binary update was performed.
- Final committed-diff review offered through the user-input prompt; no viewer opened without acceptance.
