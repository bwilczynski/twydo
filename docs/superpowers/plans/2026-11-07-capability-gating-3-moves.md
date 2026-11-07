# Capability gating 3: top-level and whole-family moves — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline. The user selected OpenCode with DeepSeek; do not start subagents. Track the checklist and return work for independent review.

**Goal:** Enable relocation of top-level tasks and flat families without demanding unrelated recurrence edit/execution verdicts.

**Architecture:** Add a move-specific capability admission path and keep the existing delete-and-recreate writer. Verify identity rewriting separately from preserved schedules and retain atomic edit-plus-move saves. Individual-item relocation remains a separate operation.

**Tech stack:** Existing Rust workspace, core gate/edit/store/queue/journal, clap, ratatui and synthetic fixture tests; no new dependencies.

**Spec:** [Incremental capability gating](../specs/2026-11-07-capability-gating-design.md); [add-and-reshape design](../specs/2026-11-03-tui-add-and-structure-design.md) §3.3 and §3.6.

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

1. A self-referential RUID must follow a new top-level UID; a foreign history/series RUID must remain foreign and unchanged.
2. Every moved child must link to the new parent in both standard and JSON representations, including independently recurring children.
3. Destination collisions, absent destinations and stale/new family members must refuse without partial source removal.
4. Mixed saves must not use move permission to bypass a refused date/repeat/type edit.
5. Recovery, queued operations and undo must not allocate fresh identities on replay or overwrite remote destination edits.

## Evidence and admission boundary

[Plan 6 H8](../../reference/phase0/plan6-baseline.md) directly records phone and Twydo family moves and phone return: `plan6-list1/07-move`, `plan6-list2/07-move`, `plan6-twydo-list2/08-move`, with the actual names confirmed in the fixture tree before referencing them. The independently recurring child keeps its previous RUID. Plan 4 and gap captures add phone-originated standalone monthly/yearly moves; their broad operation rows do not themselves establish Twydo-written moves.

Use new operation cases `standalone-resource` / `move` and `flat-family` / `move`, independently of deletion capability rows. Support unique top-level plain tasks and recognized flat checklist/project families, same structural requirements as existing safe families. Destination must exist and differ from source. Preserve all scheduling, completion, unknown properties and rank fields; change only UID, verified self uid/RUID, child parent links and modification stamps. Do not introduce an item detach/reparent bypass. Do not generalize known misread written recurrence or unknown identity metadata without documenting whether relocation exercises that mechanism; retain a narrow refusal where unresolved.

### Task 1: Audit exact identity changes and enable narrow moves

**Files:** create `crates/twydo-core/src/gate/relocate.rs`; modify `gate.rs`, `compat/matrix.toml`; tests `crates/twydo-core/tests/gate.rs`, `edit_fields.rs`, `store_family.rs`.

**Interfaces:** `relocate::check(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize, to: &str) -> Result<(), Blocked>`, called for `Op::Move` only when its dedicated row exists. Reuse `edit::moved_copy` and `Store::plan_family_move`. Do not depend on deletion-plan helpers or rows.

- [ ] Add `capability_move_rewrites_only_identity_and_stamps`: replay direct capture changes, then synthetic weekly/yearly/checklist neighbors. Compare all properties and metadata except the documented UID/self-link/parent-link/stamp changes. Assert child foreign RUID and all scheduling bytes are retained.
- [ ] Add `capability_move_flat_family_has_one_identity_map`: parent/children each get one distinct UID; new children link to the new parent; all original files disappear only after destination creates; outside-family and old history files stay byte-identical.
- [ ] Add `capability_move_gate_is_independent_of_schedule_execution`: safe schedules with unavailable execute/edit rows still move, while absent capability evidence retains legacy behavior. Missing list, same-list no-op, item move, bad links/duplicates, nesting and unknown types remain refused.
- [ ] Run `cargo test -p twydo-core --test gate capability_move` and `cargo test -p twydo-core --test store_family capability_move`; capture intended failures.
- [ ] Implement helper/dispatch and narrowly evidenced rows. Audit `Op::Save`: its extra post-edit move-row checks must use the same actual post-edit relocation contract; never leave a legacy weekly/monthly-row check blocking an admitted move or blindly skip a changed shape. If needed build an in-memory post-edit snapshot using existing overlay helpers; do not write speculative edits to disk.
- [ ] Run `cargo test -p twydo-core --test gate --test edit_fields --test store_family`; commit `feat: gate task and family moves by relocation effects`.

### Task 2: Prove atomic saves, queue, inverse and recovery behavior

**Files:** store files only if required by reproduced failures; tests `store_family.rs`, `store_queue.rs`, `journal.rs`, `crates/tdo/tests/write.rs`, `cli_op.rs`; docs `docs/compatibility.md`, `docs/tasks.md`, `docs/tui.md`, this checkpoint.

**Interfaces:** existing `Op::Save`, `Store::submit`, queue overlay, journal and list picker. Keep `m` and card List behavior; add no parent picker in this branch.

- [ ] Add `capability_move_save_checks_post_edit_shape`: title plus move is one undoable entry; a disallowed repeat/date/type plus move leaves all sources/destinations unchanged and preserves form text. A permitted type/repeat transition must revalidate the resulting family under the move contract.
- [ ] Add `capability_move_stale_family_and_destination_refuse`: changed/new/removed family member, stale source or new destination path/UID collision refuses before data loss; inspect `create` preconditions rather than assuming random UID uniqueness replaces collision checks.
- [ ] Add `capability_move_queue_reuses_prepared_identities` and `capability_move_recovery_and_undo_preserve_remote_edits`: partial creates/removals recover once under the original identity map, undo restores exact sources, and a remotely edited destination or source causes a refusal instead of overwrite. Preserve added/removed-child dependency guards through inverse/recovery.
- [ ] Add real TUI/CLI move and card-save assertions using the shipping matrix over synthetic fixtures; aggregate permission reporting still requires a proposed destination and never advertises blanket move permission.
- [ ] Fix only demonstrated writer/dependency gaps, update as-is docs with the exact moved fields and evidence distinction, run shared completion validation and commit `test: verify relocation saves and recovery`.

## Checkpoint

- [ ] Return commit IDs, evidence audit, commands and remaining exclusions for coordinator review. No device/server actions or broad verdict promotion.
