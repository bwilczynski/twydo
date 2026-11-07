# Capability gating 5: history reopening — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline. The user selected OpenCode with DeepSeek; do not start subagents. Track the checklist and return work for independent review.

**Goal:** Reopen proven history shapes by their lifecycle-only write set, keeping standalone, family and individual-child contracts distinct.

**Architecture:** Add narrow history-reopen capability paths to the existing gate and reuse the one-time/family reopen writers. Reopening a history family affects its completed children but never restores recurrence or changes the original live series.

**Tech stack:** Existing Rust workspace, core gate/edit/store/queue/journal, clap, ratatui and synthetic fixture tests; no new dependencies.

**Spec:** [Incremental capability gating](../specs/2026-11-07-capability-gating-design.md); [main spec](../specs/2026-10-31-terminal-tasks-design.md) §6; [Plan 6 H3/H7](../../reference/phase0/plan6-baseline.md).

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

1. A history copy must remain a detached non-repeating occurrence after reopen; its foreign RUID remains intact and the original live series is byte-identical.
2. Reopening a family reopens all completed children, including history children completed before the parent; open/cancelled children remain unchanged.
3. An individual child under a completed parent must not become open while its parent stays completed through an accidental gate bypass.
4. Starts, due times and alarms are preserved, not rescheduled, and a cancelled or ended live repeat must not take the history path.
5. A changed/new family member, queued dependency change or remote edit during undo/recovery must refuse atomically.

## Evidence and boundary

Direct evidence: [Plan 6 H7](../../reference/phase0/plan6-baseline.md) phone `plan6-list1/06-reopen-parent` and Twydo parent-reopen round, plus H3/child history captures and the existing narrow `history-copy.reopen` Plan 8 evidence. H7 explicitly reopens all children of the completed project copy and leaves the original live recurring project unchanged. Date/start/alarm combinations generalized through exact preservation are not new direct phone tests.

Start with completed top-level non-repeating history tasks and recognized flat history projects/checklists. Preserve UID, RUID, RELATED-TO/parentuid, all recurrence metadata, dates, alarms, ranks and unrelated content. Change only STATUS→NEEDS-ACTION, remove COMPLETED, PERCENT-COMPLETE→0 and normal stamps on reopened resources, using existing writer representations. Open/cancelled resources remain held. No ended-series-with-RRULE, cancelled reopening, restoration of recurrence, live-series rollback or independently reopening children of completed parents is admitted here.

Individual history items under open parents are a separate capability. Enable only if existing H3/child captures and preservation reasoning support the actual operation; otherwise keep refused with a precise missing-proof note. Do not infer that parent-reopen approval authorizes an isolated child reopen under a closed parent.

### Task 1: Standalone history lifecycle capability

**Files:** create `crates/twydo-core/src/gate/history_reopen.rs`; modify `gate.rs`, `compat/matrix.toml`; tests `gate.rs`, `edit_complete.rs`, `store.rs`.

**Interfaces:** `history_reopen::check(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize) -> Option<Result<(), Blocked>>`. `None` preserves legacy handling when no matching capability exists or the resource is not completed history without RRULE. Add `history-task` / `reopen` independently of the old narrow history row.

- [ ] Add `history_task_capability_preserves_schedule_and_series`: use captured date-only history and synthetic timed/start/relative/absolute-alarm variants; reopen only the selected file and compare the exact permitted lifecycle/stamp deltas. The live original and all unrelated files remain byte-identical; foreign RUID remains unchanged.
- [ ] Add `history_task_capability_refuses_live_cancelled_and_ended_series`: duplicate/unreadable identities, own RRULE, cancelled status, unknown type and absent/unsupported capability evidence do not enter the path. Existing ended-until behavior and guards remain intact.
- [ ] Run `cargo test -p twydo-core --test gate history_task_capability` and `cargo test -p twydo-core --test edit_complete history_task_capability`; confirm intended safe admission fails before code changes.
- [ ] Implement helper/dispatch without bypassing structural validation. Record the evidence/generalization on the dedicated row and in docs; retain the legacy path for old matrices.
- [ ] Run core gate/edit/store targets, then commit `feat: admit history task reopening by lifecycle effects`.

### Task 2: Whole-family history and separately bounded item history

**Files:** same helper/gate/matrix; tests `gate.rs`, `store_family.rs`; existing `Store::plan_family_reopen` and `edit::reopen_member` only if a captured mismatch is reproduced.

**Interfaces:** dedicated `history-flat-family` / `reopen`, plus `history-item-open-parent` / `reopen` only if evidence is sufficient. Preserve legacy per-family operation safeguards. Parent-driven reopening of children is one family operation, not recursive per-item gate admission.

- [ ] Add `history_family_capability_matches_h7`: parent and every completed child reopen once, including previously completed/history children; open/cancelled members stay unchanged, and the live series plus its live children are untouched. Compare to the actual captured H7 write set.
- [ ] Add `history_family_capability_refuses_unsafe_relationships`: nested, plain-task parent with children, cross-list/disagreeing links, duplicate identity and unknown member types refuse. A member's unrelated schedule-execution row cannot block a lifecycle-only history operation, but malformed/ambiguous recurrence metadata cannot be guessed.
- [ ] Add `history_item_capability_keeps_parent_policy`: under an open parent, a proven history-child reopen writes only that child and holds its parent/dependencies; under a completed/cancelled parent, isolated reopen remains refused. If no sufficient direct evidence supports open-parent history items, assert shipping refusal and report it separately.
- [ ] Run new tests before implementing each path. Add only supported dedicated rows; do not simply delete `is_history_copy` or family guards to let ordinary one-time rows bypass evidence.
- [ ] Run `cargo test -p twydo-core --test gate --test store --test store_family --test edit_complete`; commit `feat: scope history family reopening explicitly`.

### Task 3: Queue/inverse/recovery and UI reporting

**Files:** tests `store_family.rs`, `store_queue.rs`, `journal.rs`, `crates/tdo/tests/write.rs`, `cli.rs`, `cli_op.rs`; store dependency fixes only if required; docs `docs/tasks.md`, `docs/tui.md`, `docs/compatibility.md`, this checkpoint.

- [ ] Add `history_reopen_stale_family_and_inverse_refuse`: newly added/removed/changed members and remotely edited live/history resources are preserved; no stale inverse overwrites them. Verify dependencies needed by the family survive queue/recovery/inverse, rather than only an initial apply.
- [ ] Add `history_reopen_queue_and_recovery_are_idempotent`: queued overlay matches final reopened resources; interrupt after one family update, recover once and undo to exact originals with no new series or copied identities.
- [ ] Add real completed-section TUI selection and CLI/aggregate permission assertions; allowed/refused counts agree with the gate, no task content leaks into default reports, and family reopen remains one undo step.
- [ ] Update docs distinguishing reopening history from undoing a recurrence advancement and from ended live-series reopen. Run shared completion validation; commit `test: cover history reopening workflows and recovery`.

## Checkpoint

- [ ] Return exact enabled standalone/family/item paths, unresolved child-policy scope, evidence, commit IDs and validation. No production reopening or phone round.
