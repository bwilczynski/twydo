# Capability gating 4: recurring-checklist scheduling edits — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline. The user selected OpenCode with DeepSeek; do not start subagents. Track the checklist and return work for independent review.

**Goal:** Enable supported recurring-checklist due-date and interval edits through separate scheduling contracts, retaining inherited-item semantics.

**Architecture:** Audit captured field transitions first, then add dedicated due-edit and interval-edit admission. Reuse the checklist-date cleanup planner and recurrence field writer; preserve current execution gates rather than treating scheduling edits as text preservation.

**Tech stack:** Existing Rust workspace, core gate/edit/store/queue/journal, clap, ratatui and synthetic fixture tests; no new dependencies.

**Spec:** [Incremental capability gating](../specs/2026-11-07-capability-gating-design.md), proportional verification; [main spec](../specs/2026-10-31-terminal-tasks-design.md) §6–7.

**Execution:** Independent branch from the committed planning checkpoint after #60. Other capability branches are integrated and reviewed by the coordinating agent. Read the project AGENTS.md and linked specs first. User approval covers local implementation and separate commits, not device verification or publication.

## Global constraints

- Implement only this plan in its assigned worktree. Other capability plans are separate branches; do not merge, rebase, push, start agents or change another worktree.
- No production task, config, state, server, NAS or phone access. All runtime verification uses temporary synthetic vdirs with isolated XDG_CONFIG_HOME and XDG_STATE_HOME and sync disabled.
- Reuse existing captures before asking for interoperability work. Additional phone cases must name a failure mechanism that captures/local tests cannot answer; document a bounded proposal and leave that capability refused rather than inventing evidence.
- Do not mark broad operation/rule rows passing. Add narrowly scoped operation cases with precise evidence notes; distinguish directly observed behavior from preservation generalization. Missing/custom capability rows retain existing behavior.
- Preserve ambiguous-identity, malformed-link, unknown-type, nesting, conflict, stale, strict queued-file, journal recovery and undo safeguards. Unchanged scheduling is not recurrence execution or recurrence editing.
- No personal data in code, fixtures, docs, commits or reports. Use synthetic existing fixtures and generated identities only. No agent attribution in commits. Make focused implementation commits after verification.
- Existing item-deletion, ordering, RF/RA and C29 phone rounds are complete and must not be repeated. Do not alter account or global write mode.
- A mixed save passes every changed field's contract independently, stays atomic, and produces one undo entry. Permission reports continue to call the real gate; do not add a second permission table.

## Validation shared by every task

Use existing behavior test targets, not session-named targets. Reuse `tests/common::Env`, synthetic captures and existing queue/journal helpers. Record a failing assertion before changing behavior, then run the affected targets. At completion run `cargo test --workspace`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`. Use a disk-backed target directory outside the worktree; do not fill /tmp with build artifacts. Report exact command results and any evidence-dependent exclusions.

## Review focus

1. Moving a checklist due date must remove exactly the verified inherited item-date representations and preserve all unrelated child content.
2. Interval-only full Repeat values from the edit card must match CLI RepeatInterval admission and resulting bytes.
3. Text-only, due-only, interval-only and mixed saves need distinguishable contracts; a passing edit does not imply completion of the resulting schedule.
4. Existing child-owned dates, cancelled members, starts, due times, alarms and endings must not get guessed cleanup or advancement semantics.
5. Stale/new members and partially recovered date cleanup must not let the parent and items diverge.

## Evidence and scope

Read [Plan 6 checklist-date evidence](../../reference/evidence.md), `checklist-item-dates` in the matrix, `fixtures/phase0/v1/plan6-checklist-date-list1/01-twydo-date` and `02-phone-date`; also [Plan 8 ledger](../../reference/evidence.md) and C29. The dedicated item-date cleanup edge passes already. `captured_checklist_write` currently rejects ordinary due and interval edits before this existing writer can run. Existing checklist repeat rows cover particular monthly/weekly interval-1 transitions, not arbitrary intervals or execution of them.

First target: open, top-level, readable JSON flat checklists; plain due-anchored monthly date-only due edits with resulting day 1–28, no start/time/alerts/end/delay, and simple inherited or undated plain items. A separate interval-only path initially targets captured/provable monthly interval 1→2 with due-day 1–28 and unchanged anchor/end. If existing captures do not support that interval write contract, implement/test its candidate behind an unverified dedicated row and explicitly report the missing evidence. Do not remove this boundary to make the plan appear complete.

Do not add start/time/removal or arbitrary repeat transitions in this increment. A checklist item's independent date/start remains refused. Unchanged text may compose with a proven scheduling contract; each changed scheduling field must qualify. Other repeat removals and project/child rule edits stay separate.

### Task 1: Document field evidence and implement date-only due edits

**Files:** create `crates/twydo-core/src/gate/checklist_edit.rs`; modify `gate.rs`, `compat/matrix.toml`; tests `gate.rs`, `edit_fields.rs`, `store_family.rs`; create `docs/reference/phase0/checklist-scheduling-capability.md` as a local evidence audit, not a new phone session.

**Interfaces:** `checklist_edit::check(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize, changes: &[Change]) -> Option<Result<(), Blocked>>`. `None` means outside its narrow due/interval contract and uses the existing gate. `Some` decides only the proposed field subset; normal identity/family/known-rule safeguards still run. Use dedicated `recurring-checklist-due` / `edit` evidence, not broad recurring-checklist.edit.

- [x] Record direct captured writes, field/resource deltas and preservation reasoning for adding unchanged recurrence to the existing checklist due operation. Explain whether checklist date cleanup removes dates on any dated item or only inherited copies; retain the existing verified writer behavior for the supported input boundary.
- [x] Add `checklist_due_capability_changes_parent_and_cleans_inherited_items`: move due by one day, compare parent representations and item date cleanup against captures, assert metadata/status/repeat/alarms/unknown fields elsewhere are unchanged. Include empty and nonempty checklists.
- [x] Add `checklist_due_capability_refuses_negative_neighbors`: day29–31 target, missing due, timed/zoned due, start/time/alarms/end/delay input, independently repeating/nested/malformed item, unknown metadata or mismatched links, unsupported mixed change and a missing evidence row all stay refused.
- [x] Run `cargo test -p twydo-core --test gate checklist_due_capability` and `cargo test -p twydo-core --test store_family checklist_due_capability`; confirm new safe admission fails first.
- [x] Implement narrow due dispatch after ordinary structural validation but before unrelated captured-text restrictions. Reuse `checklist_date_change` and `Store::plan_checklist_date`; do not suppress date-cleanup guards or skip mixed-save field validation.
- [x] Run `cargo test -p twydo-core --test gate --test edit_fields --test store_family`; document the dedicated row/evidence only when justified and commit `feat: admit verified recurring checklist due edits` (or accurately label candidate-only support if unresolved).

### Task 2: Separate interval editing from execution

**Files:** same helper/gate/matrix; tests `gate.rs`, `edit_fields.rs`, `store_family.rs`, `crates/tdo/tests/write.rs`, `cli_op.rs`; evidence audit above.

**Interfaces:** same helper; add `recurring-checklist-interval` / `edit` row. Normalize an interval-only full `Change::Repeat` by comparing all other rule/anchor/end fields, so it has the same admission as `RepeatInterval`; no new parser.

- [x] Add `checklist_interval_capability_cli_and_card_are_equivalent`: the CLI interval-2 and card full monthly interval-2 representations produce the same parent recurrence fields, with all child bytes and other fields intact. Assert a resulting interval-2 checklist still has its actual existing completion decision; this edit cannot grant execution.
- [x] Add `checklist_interval_capability_rejects_anchor_end_and_class_changes`: changes to anchor/count/until/BY fields, mixed incompatible dates, missing dedicated evidence and unsupported intervals remain refused. Test with explicit candidate matrices if shipping evidence is insufficient; shipping tests must then assert refusal.
- [x] Run the new tests first; implement only the proven contract, retaining recurrence metadata validation and written-interval constraints. Update the evidence document to say precisely whether admission is shipping or candidate-only, and why.
- [x] Run affected core/CLI/TUI targets and commit `feat: separate checklist interval edit capability` (use an accurate test/candidate commit type if not enabled).

### Task 3: End-to-end dates, dependencies and docs

**Files:** `store_family.rs`, `store_queue.rs`, `journal.rs`, `crates/tdo/tests/write.rs`, `cli_op.rs`; writer fixes only if tests expose gaps; docs `docs/tasks.md`, `docs/tui.md`, `docs/compatibility.md`, this checkpoint.

- [x] Add `checklist_schedule_stale_members_refuse_atomically`: new/changed/removed items after form open or queueing cause refusal, preserve typed text and leave parent/items unchanged.
- [x] Add `checklist_schedule_inverse_and_recovery_keep_dates_consistent`: interrupted parent/child cleanup, retry, undo/redo and remote edits follow existing journal contracts with exact identity/date preservation. Interval edit stays one parent-resource update.
- [x] Add TUI date-key and card-field tests, including cancelled edit, unsupported mixed save and repeat completion after a permitted due edit; independently dated checklist-item shortcuts remain refused.
- [x] Update matching docs with enabled fields, unchanged completion boundary, excluded scheduling shapes and any missing bounded evidence. Run shared completion validation; commit `test: verify checklist scheduling workflows and recovery`.

## Checkpoint

- [x] Return enabled vs candidate-only paths, direct vs generalized evidence, commits and verification. Do not schedule or conduct a phone experiment.


## Integrated closeout (2026-11-08)

Implemented and integrated on `feat/completness`; see the
[capability implementation review](../../reference/evidence.md)
for root-branch commit IDs, all final validation results, review corrections,
execution decisions and remaining shipping refusals. The user's later instruction
superseded the original OpenCode executor constraint. Implementation commits were
grouped by behavior where shared prerequisites made that clearer; plan 4's due,
interval candidate and workflow tests share one feature commit. Local synthetic
candidate coverage does not promote missing interoperability evidence. Final review
was performed by the author because independent review agents were unavailable.
