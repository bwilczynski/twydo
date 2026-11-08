# Capability gating 2: standalone and whole-family deletion — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline. The user selected OpenCode with DeepSeek; do not start subagents. Track the checklist and return work for independent review.

**Goal:** Admit exact resource removal independently of unchanged recurrence schedules while retaining family and identity safeguards.

**Architecture:** Add a deletion-specific capability path to the existing gate, after the established individual-item deletion path. Reuse the existing removal planners and journal; change writers only if a reproduced preservation or dependency test exposes a defect.

**Tech stack:** Existing Rust workspace, core gate/edit/store/queue/journal, clap, ratatui and synthetic fixture tests; no new dependencies.

**Spec:** [Incremental capability gating](../specs/2026-11-07-capability-gating-design.md), especially policy and following increments; [main spec](../specs/2026-10-31-terminal-tasks-design.md) §6.

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

1. Deleting a live series must leave unrelated history resources and foreign-series identities untouched.
2. Empty, last-item, completed and cancelled family members must have the same exact selected-family removal boundary.
3. Raw duplicate UIDs, multiple/folded parent links and newly arrived children must not evade lookup or family rescans.
4. Inverse operations and recovery must reject changed dependencies without resurrecting or overwriting remote edits.
5. Old/custom matrices must retain their previous behavior, and the existing item-delete capability must retain its precedence.

## Evidence and admission boundary

Direct evidence: [Plan 6 H9](../../reference/phase0/plan6-baseline.md) and `fixtures/phase0/v1/plan6-list2/08-delete`, `plan6-list1/08-delete`, `09-delete-rest`; standalone monthly/yearly deletion in Plan 4/gap captures. [Issue 27](../../reference/phase0/issue27-item-deletion.md) proves individual removal and recurring empty-parent retention, not whole-family deletion by itself.

New cases: `standalone-resource` / `delete` and `flat-family` / `delete`. A standalone resource has unique identity, no parent and no children. A family has a recognized top-level checklist/project and flat plain-task children, each unique, with unambiguous same-list standard/JSON links. Use existing structural validation or strengthen it locally; a known label in the parsed model is insufficient if raw links are ambiguous. A nonempty plain-task parent remains refused. Recognized recurrence/date/alarm/status fields are not executed or rewritten. Unknown properties remain opaque. Keep unrecognized/malformed structural or repeat shapes refused in this first increment. Document exact generalized support without claiming direct phone coverage for every shape.

### Task 1: Prove and encode the removal capability

**Files:** create `crates/twydo-core/src/gate/delete.rs`; modify `crates/twydo-core/src/gate.rs`, `compat/matrix.toml`; tests `crates/twydo-core/tests/gate.rs`, `store_family.rs`, `store.rs`.

**Interfaces:** child module `delete::check(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize) -> Result<(), Blocked>`. The caller dispatches only top-level `Op::Delete` when the matching new capability row exists. Existing item deletion and custom matrices without the row use their existing paths. The helper validates identities and structure before checking its dedicated operation row.

- [x] Add `capability_delete_removes_only_selected_resource_or_flat_family`: with the shipping matrix, weekly standalone, empty weekly project, empty/nonempty recurring checklist and completed flat families pass deletion; only the selected UID and actual child resources disappear. Derive synthetic neighbors from existing captures.
- [x] Add `capability_delete_preserves_series_history_and_other_resources`: live repeating parent and unrelated history copies coexist; deleting the live family leaves history resources and every outside-family byte unchanged. Include date/start/relative/absolute-alarm and count/until metadata as preserved payload, not execution claims.
- [x] Add `capability_delete_rejects_unsafe_membership`: duplicate target/child UID, multiple or disagreeing parent links, cross-list child, unknown type, nested family and plain-task parent are refused. Existing individual-item deletion assertions remain passing.
- [x] Add `capability_delete_requires_its_own_evidence`: missing/unverified/unsupported new rows and custom matrices retain expected legacy behavior; execution/edit verdicts do not authorize removal. No other operation becomes admitted because of these rows.
- [x] Run `cargo test -p twydo-core --test gate capability_delete` and `cargo test -p twydo-core --test store_family capability_delete`; confirm the intended admission tests fail before implementation.
- [x] Implement the helper and minimal dispatch. Add the two dedicated rows only after the captured invariant and local preservation tests support them; cite direct captures and generalization separately. Reuse `operation_row` semantics, including test-mode baseline requirements.
- [x] Run the focused tests, then `cargo test -p twydo-core --test gate --test store --test store_family`; commit `feat: admit deletion by exact resource scope` with explicit paths.

### Task 2: Pin durable dependencies and user-visible behavior

**Files:** modify store/family/journal code only for reproduced failures; tests `crates/twydo-core/tests/store_family.rs`, `store_queue.rs`, `journal.rs`; `crates/tdo/tests/write.rs`, `cli.rs`, `cli_op.rs`; docs `docs/compatibility.md`, `docs/tasks.md`, `docs/tui.md`, this checkpoint.

**Interfaces:** existing `Store::submit`, journal recovery, undo/redo and selected-row `dd`; no new CLI/TUI operation or permission-probe interface.

- [x] Add `capability_delete_refuses_changed_family_before_commit`: changed/removed/new child, changed parent and unrelated concurrent edit; affected family changes refuse atomically while unrelated edits remain preserved.
- [x] Add `capability_delete_queue_and_inverse_keep_scope`: queued removal has strict file/dependency guards; success overlays match committed removals; stale undo/redo does not overwrite a remotely edited/restored member or new family member.
- [x] Add `capability_delete_recovery_is_idempotent`: interrupt a multi-file removal after the first removal and after all removals; recovery finishes once, undo restores exact originals, unrelated history remains identical. Reuse existing interruption helpers.
- [x] Add CLI/TUI tests selecting the parent (including empty/completed sections): `dd` uses whole-family scope and one undo; `tdo check` and `op check` reflect the gate without titles in default aggregate output. A refused deletion writes no bytes.
- [x] Run the affected targets before and after any necessary dependency fix. Update the matching as-is docs with direct evidence, generalized scope and exclusions.
- [x] Run shared completion validation, inspect the staged diff for private data and commit `test: cover deletion recovery and user workflows` (include any necessary fix and docs).

## Checkpoint

- [x] Implementation, evidence notes, validation and self-review complete.
- [x] Return commit IDs, tests, remaining refusals and any unresolved proof to the coordinator. Do not push or perform a device round.


Direct-execution closeout: the reviewed deletion path additionally rejects JSON-only or
mismatching family references before admission and when they arrive after the snapshot.
`FamilyGuard.parent_aliases` retains RFC/internal identities through deletion, queue,
inverse and recovery; old guard JSON remains readable. Parsed links pointing to an internal
alias in place of an RFC parent link are refused as ambiguous. The formerly "unrelated"
fixture was corrected to update both parent representations, including folded metadata.
Unknown numeric/string task types are refused without losing the reader's supported valid
integer representations. Whole-family removal includes history linked as children and
preserves history outside the selected family. No new device evidence was claimed.

Validation: full offline Rust workspace, 49 Python tests, documentation links, clippy with
warnings denied, formatting and whitespace checks passed. This increment remains subject
to the final combined-branch review before publication; no push or production write occurred.


## Integrated closeout (2026-11-08)

Implemented and integrated on `feat/completness`; see the
[capability implementation review](../../reference/phase0/capability-implementation-review.md)
for root-branch commit IDs, all final validation results, review corrections,
execution decisions and remaining shipping refusals. The user's later instruction
superseded the original OpenCode executor constraint. Implementation commits were
grouped by behavior where shared prerequisites made that clearer; plan 4's due,
interval candidate and workflow tests share one feature commit. Local synthetic
candidate coverage does not promote missing interoperability evidence. Final review
was performed by the author because independent review agents were unavailable.
