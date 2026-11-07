# Capability gating 6: item relocation and indent/outdent — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan inline. The user selected OpenCode with DeepSeek; do not start subagents. Track the checklist and return work for independent review.

**Goal:** Implement explicit parent-membership changes and TUI indent/outdent behind their own evidence gates, preserving existing list-move semantics.

**Architecture:** Introduce a dedicated core reparent operation and journal planner with source/destination family guards. Reuse it for CLI, list/parent picker and >>/<<. New relationship encodings stay disabled in shipping verified mode until representative interoperability evidence is established; this plan authorizes local implementation only.

**Tech stack:** Existing Rust workspace, core gate/edit/store/queue/journal, clap, ratatui and synthetic fixture tests; no new dependencies.

**Spec:** [Add-and-reshape design](../specs/2026-11-03-tui-add-and-structure-design.md) §5; [incremental capability gating](../specs/2026-11-07-capability-gating-design.md); main spec recovery/compatibility contracts.

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

1. Both old and new parents, their member sets and the selected leaf must stay current through queueing, commit, undo/redo and crash recovery.
2. No operation may create nesting, cycles, cross-list orphan links or conflicting standard/JSON parent identities.
3. Checklist inbound date loss requires a cancellable confirmation; checklist outdent yields an undated plain task and must not invent recurrence or alarms.
4. Identity/foreign-series behavior for a relationship change is not proven by a list move; candidate writers must not become shipping permissions accidentally.
5. Keys apply only in the designed contexts; failed/refused/cancelled relocation preserves focus, form text and all task bytes.

## Evidence barrier and bounded first slice

The TUI behavior is already designed: `>>` in a list view makes an open leaf the last item of the nearest preceding eligible parent in that same list; `<<` detaches an item into the same list; a checklist item comes out undated. The `m` picker can select a list or a project/checklist nested under it. Into a checklist, losing dates asks first; Escape always cancels. Parent-with-children relocation is not indent and remains the existing whole-family list move.

No existing item move row is verified, and this plan grants no NAS/phone access. Start by auditing existing captures and exact metadata. If they do not establish identity/link and checklist-date behavior, add explicit unverified rows and build synthetic candidate tests using custom matrices. Never label guessed output as an iPhone capture or pass shipping rows just to expose the keys. Return a short bounded verification proposal naming the unresolved mechanisms; the coordinator/user must approve any later device work. Local candidate functionality plus truthful shipping refusal is a valid completion state, clearly reported as such.

Initial scope: one open plain leaf, recognized open top-level destination checklist/project, no nested families. For project membership preserve the leaf's dates/repeat/alerts. For checklist membership, the candidate may clear supported simple date representations after confirmation; own recurrence/alarms/unsupported metadata remain refused pending a separate established cleanup contract. Completed/cancelled reparenting and parent-with-children reparenting remain excluded. Cross-list item-to-list and item-to-parent share the explicit relationship operation, not the old Move bypass.

### Task 1: Define the operation and implement candidate relationship edits

**Files:** modify `crates/twydo-core/src/edit/mod.rs`; create `edit/reparent.rs`, `gate/reparent.rs`; modify `gate.rs`, `compat/matrix.toml`; tests existing `edit_family.rs`, `gate.rs`; document evidence audit at `docs/reference/phase0/item-relocation-capability.md`.

**Interfaces:** add serializable `Op::Reparent { uid: String, collection: String, parent: Option<String> }`: `parent=None` means detach into the explicit collection, `Some(uid)` means attach to that top-level parent in that collection. Existing `Op::Move` still means whole top-level task/family relocation and still refuses a single item. Candidate field writer `edit::reparented_copy(cal: &Calendar, new_uid: &str, parent: Option<&str>, clear_dates: bool, now: DateTime<Utc>) -> Result<Calendar, EditError>` must have its identity policy documented from evidence; if delete/recreate is not established, keep this writer candidate-only and name the missing proof. Preserve foreign RUID, rewrite only documented self identity and parent representations; remove parent representations on detach. Never silently retain the former RELATED-TO on an outdented resource.

- [ ] Audit direct captures for any member-move representation; distinguish a list-move's recreated UID/self/foreign RUID rules from an actual reparent. Document every proposed identity/link/date change and missing proof.
- [ ] Add `reparent_candidate_link_representations_agree`: attach/detach have matching standard/JSON parent links, deterministic single identity map and preserved foreign series links; raw duplicates/disagreements/unreadable metadata refuse.
- [ ] Add `reparent_candidate_checklist_dates_are_explicit`: attaching a simple dated leaf clears only the supported candidate date fields; an undated leaf has no invented dates; detached checklist item is undated; recurrence/alarms requiring unverified cleanup refuse.
- [ ] Add `reparent_shipping_requires_dedicated_evidence`: dedicated `item-to-project`, `item-to-checklist`, `item-outdent` operation cases with op `reparent` must not admit writes without applicable evidence, including test mode's baseline requirement. Tests with candidate matrices exercise the code without promoting shipping rows.
- [ ] Run new tests and confirm the missing operation/behavior fails, implement exact operation/writer/gate, and update all exhaustive Op matches without inventing default behavior for queue or Store.
- [ ] Run gate/edit-family/serde-related tests; commit `feat: model gated item reparenting`.

### Task 2: Journal relocation with two-family dependencies

**Files:** create `crates/twydo-core/src/store/reparent.rs`; modify `store/mod.rs`, `store/family.rs`, `store/journal.rs`, queue handling only as required; tests `store_family.rs`, `store_queue.rs`, `journal.rs`.

**Interfaces:** `Store::plan_reparent(&self, snap: &Snapshot, uid: &str, collection: &str, parent: Option<&str>, now: DateTime<Utc>) -> Result<Plan, WriteError>`. Plan the entire change once with stable IDs; hold both old/new parent bytes and member sets. Existing journal currently persists one optional FamilyGuard: extend it additively with `#[serde(default)]` multi-family dependencies if needed, keeping old journal/queue files readable and validating every dependency before commit/inverse/recovery. Do not overload one guard and lose the other family.

- [ ] Add `reparent_store_updates_exact_membership_once`: source removed/replaced according to the candidate identity contract, destination member links correct, old/new parents and other siblings byte-identical, ranks append at destination using existing rank conventions. One operation/undo step.
- [ ] Add `reparent_store_checks_both_families`: changed/deleted old or new parent, new/removed/changed sibling on either side, target edit, destination path collision and mixed raw identities all refuse without partial writes.
- [ ] Add `reparent_queue_inverse_and_recovery_keep_both_guards`: prepared queue retains both dependency sets and stable IDs; interrupt after create/before remove, retry once, undo/redo restore exact identities/bytes; remote modifications remain protected. Deserialize old journal/queue fixtures unchanged.
- [ ] Run new tests before implementing dependency/planner changes. Keep compatibility changes minimal and documented; do not relax strict whole-file checks or implement queue rebasing in this branch.
- [ ] Run store-family/store-queue/journal plus complete core tests; commit `feat: journal item relocation with both parent guards`.

### Task 3: Expose CLI and TUI structure commands with truthful refusal

**Files:** modify `crates/tdo/src/cli/op.rs`, `cli/permissions.rs`, `app.rs`, `write.rs`, `ui/picker.rs`, `ui/status.rs` and key/help handling where it actually lives; tests `crates/tdo/tests/cli_op.rs`, `write.rs`, `app_keys.rs`; docs `docs/tui.md`, `docs/tasks.md`, `docs/compatibility.md`, `docs/architecture.md`.

**Interfaces:** add CLI `tdo op reparent <uid> --list <existing-list-id> [--parent <uid>]` using the new Op and existing queue/--wait handling. Extend the picker target representation to distinguish list and parent destinations, without treating a parent UID as a collection id. Permission probes report reparent needs destination/context, never blanket permission.

- [ ] Add `structure_keys_indent_to_nearest_parent_in_list`: `>>` only in list views, nearest eligible previous same-list parent, append position; no preceding parent or a smart view yields an explanatory refusal. Exclude selected parent-with-children and nesting.
- [ ] Add `structure_keys_outdent_and_checklist_confirmation`: `<<` detaches to the same list; checklist output undated; inbound date loss requires confirmation; Escape/No writes nothing and preserves selection. Unsupported cleanup and shipping unverified capability refuse visibly.
- [ ] Add `structure_picker_distinguishes_list_and_parent`: filters nested targets, excludes self/invalid parents, list choices preserve existing top-level Move behavior, item choices use Reparent. No new-list creation or yank/paste in this increment.
- [ ] Add CLI serialization/queue tests and truthful `op check`/aggregate needs-input labels. Refused writes leave journal/task bytes unchanged under shipping matrix; candidate matrix tests exercise successful commands without user data.
- [ ] Implement the thin CLI/UI routing and help text, run focused CLI/TUI tests, update docs with candidate vs verified availability and explicit exclusions, run shared completion validation and commit `feat: add gated indent outdent and parent destinations`.

## Checkpoint

- [ ] Return implemented paths and shipping-enabled paths separately, exact evidence gaps, a minimum proposed verification checklist, commits and tests. No phone uploads, synthetic-list cleanup or unapproved evidence promotion.
