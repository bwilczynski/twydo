# Issue 27: Individual Item Deletion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable selected-item deletion using four representative phone examples and general preservation tests.

**Architecture:** Extend Plan 8. Establish the deletion write-set invariant on the phone, reproduce it through the existing journal/dependency/queue path, then generalize operation admission independently from recurrence execution. Reuse existing capture helpers and behavior-based test targets.

**Tech Stack:** Rust/Cargo, Python fixture tools, scoped vdirsyncer, 2Do iPhone.

**Spec:** [approved four-example session](../../reference/evidence.md), under [main spec](../specs/2026-10-31-terminal-tasks-design.md) §6/§9.

## Global Constraints

- No personal data in this repository; private state and backups outside git.
- Exactly four paired examples, eight families, 20 starting resources. The oversized session is superseded and its candidates discarded.
- Only Twydo test; keep its collection and every other list/config/account intact.
- Initial contents cleanup was explicitly authorized and completed; no further cleanup is assumed.
- Two phone phases; separate deletion/completion/return capture barriers.
- Shared vdir/state/lock, no forced deletion or global write-mode changes.
- Preserve all unrelated bytes; deletion and recurrence execution have distinct verdicts.

## Review Focus

- Last-item deletion may alter parent type/status/rule: use phone deltas, not assumptions.
- An independently recurring child must be removed without advancing it or its parent.
- Completed-parent child selection must resolve the actual child UID, not a parent or live-series counterpart.
- Durable family dependencies must survive inverses, recovery and remote edits.
- Queue admission must retain strict whole-file/membership checks and refuse stale deletes safely.

### Task 1: Prepare/upload the four examples and capture the phone baseline

**Files:** session record; private manifest/receipts; vetted captures under `fixtures/phase0/v1/issue27-items/`.

**Interfaces:** Consumes ca/pa/ma/wa and cb/pb/mb/wb aliases; produces exact before/after phone resource sets and UID/hash mappings.

- [x] Create approved Herdr worktree; baseline `cargo test --workspace`: 1,030 passed.
- [x] Back up current vdir/state/config and exact remote resources; approved test-list clear and shared-runner reconciliation complete. Phone confirmed empty after reopening.
- [x] Reduce the rejected matrix to four examples; prepare and export-validate exactly 20 synthetic records offline, including completed x and independently repeating project q.
- [x] Fresh scoped empty/collision check; create/upload only these 20 records under the shared lock and verify read-back/other-list hashes. Expected: exactly 20 records, starting states match the manifest.
- [x] Run phone 010, 020 and 030 separately, capturing after each batch. Expected resource counts: 18, 18, 14 if deletion is uniform; inspect actual parent/item changes.
- [x] Scan only new synthetic captures for privacy and commit vetted baseline evidence and session results.

### Task 2: Implement the verified deletion invariant

**Files:** `crates/twydo-core/src/gate.rs`, `compat/matrix.toml`; `crates/twydo-core/src/store/mod.rs` and `store/family.rs` only if needed; existing `tests/gate.rs`, `store_family.rs`, `store_queue.rs`, `journal.rs`.

**Interfaces:** Retain `gate::check(&Snapshot, &Matrix, WriteMode, &Op, NaiveDate) -> Result<(), Blocked>` and journal compatibility. Consumes observed write sets; produces exact item-delete plans with durable family dependencies.

- [x] Add failing tests for open/completed/last-item deletion, closed parents, independently recurring children and general recognized recurrence/date/alert preservation. Expected RED: current gate refuses newly supported shapes.
- [x] Generalize dedicated item-delete admission to the proven invariant; do not enable parent deletion or recurrence execution through the item row. Preserve malformed/duplicate/broken-link/nested refusals.
- [x] Match phone write sets exactly, reusing the single-resource removal writer if verified. Retain durable parent/sibling holds and membership guards through apply/inverse/recovery.
- [x] Add failing regressions for stale/added/removed family members, history-vs-live identity, unknown content, undo/redo after sync and interrupted recovery. Implement minimally and verify GREEN.
- [x] Run `cargo test -p twydo-core --test gate --test store_family --test store_queue --test journal`; commit focused implementation/test changes. Shipping verdicts stay unpromoted until return evidence.

### Task 3: Candidate acceptance, TUI checks and closeout

**Files:** candidate/return captures and matrix provenance; existing TUI action tests/code; `docs/tui.md`, `docs/tasks.md`, `docs/compatibility.md`, Plan 8 checkpoint and coverage ledger.

**Interfaces:** Existing selected-child `dd`, `u`, Ctrl-r and strict queue APIs; no new keys. Consumes Task 2 plans and unchanged b twins; produces bidirectional evidence and finalized deletion verdicts.

- [x] Fresh scoped pull; replay the seven operations on b twins with individual captures and exact dependency checks, then upload. Expected: eight retained parents and no children if uniform deletion is verified.
- [x] Phone 040: inspect all b parents and change notes x→z; capture actual phone writes. Promote only evidence-backed item-delete rows.
- [x] Add RED→GREEN verified-mode TUI tests for selected and completed child deletion, last-item rendering, undo/redo and queued deletion. Update only necessary product code.
- [x] Update as-is docs and Plan 8 coverage, stating the generalized operation invariant and exact malformed/unsupported neighbors.
- [x] Run focused checks, then `mise run test`, `mise run lint`, `python3 tools/check_links.py`. Expected: all pass; verify staged privacy and write sets.
- [x] Review the branch and fix material findings with regressions; make focused commits. Offer final diff review without opening a viewer unasked. Report results; no push/merge/post/cleanup assumed.
