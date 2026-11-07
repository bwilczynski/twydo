# Issue 41: checklist/project item reordering — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for inline execution or superpowers:subagent-driven-development if requested. Track steps with checkboxes.

**Goal:** Persist checklist/project sibling moves and anchored insertions in shared `DisplayOrder` metadata.

**Architecture:** Core operations build one journaled family change using the existing sibling comparator. Submit through the write queue added on main, with durable parent/sibling guards. The TUI resolves visible neighbors and preserves the selected UID.

**Tech Stack:** Existing Rust workspace, ratatui, serde JSON metadata editor; no new dependencies.

**Spec:** [Approved issue 41 design](../specs/2026-11-06-issue41-item-reorder-design.md).

## Global constraints

- Rebased onto main `9e5f1d7` before implementation; retain its queue/overlay/drain behavior.
- No sorting controls, top-level ordering, reparenting or indent/outdent.
- Preserve dates, recurrence, completion, eligibility, counts and unrelated metadata.
- Device/server access requires fresh bounded approval. Never change accounts or global write mode.
- Phone-side reordering is unavailable; verify Twydo-written display order and preservation through a phone edit instead.
- Do not commit personal data or duplicate unchanged fixture resources. No agent attribution.
- Keep changes uncommitted while a live working-tree diff review is running.

## Review focus

1. A queued second move must plan against the overlay and drain in order without losing either move.
2. A hidden or detached sibling must never become an unrelated move target; full-family order stays deterministic.
3. A new sibling or modified parent during queueing/recovery must refuse the whole stale action.
4. Legacy ties require explicit ranking only on moves/anchored insertion; ordinary loads and appends remain unchanged.
5. Unsupported phone-originated reorder must not disable Twydo-originated verification or weaken other compatibility gates.

### Task 1: Core operations, compatibility gate and durable ordering writes

**Files:** create `crates/twydo-core/src/store/order.rs`; modify `edit/mod.rs`, `gate.rs`, `store/mod.rs`, `store/family.rs`, `compat/matrix.toml`; extend `tests/store_family.rs`, `tests/store_queue.rs`, `tests/gate.rs`, `tests/journal.rs`. Update exhaustive operation matches in `crates/tdo/src/write.rs` as needed to keep the workspace compiling.

**Interfaces:** Add `edit::Placement::{Before, After}`, `Op::Reorder { uid: String, anchor: String, placement: Placement }`, and `Op::CreateAt { task: NewTask, anchor: String, placement: Placement }`. Keep `Op::Create(NewTask)` as the append operation, avoiding changes to existing constructors. Add `Store::plan_order(&self, snap: &Snapshot, op: &Op, now: DateTime<Utc>) -> Result<Plan, WriteError>` in the new store module. Consume `views::compare_items`, `MetaJson`, `Prepared`, journal holds and family guards.

- [x] Add failing tests for both parent types: A,B,C -> move C before A -> C,A,B; undo restores exact original bytes; redo restores reordered bytes. Check all unrelated logical fields/metadata and creation timestamps, and unchanged resources whose ranks already match. Cover zero/tied/missing/malformed/sparse/maximal positions and tied/missing creation times, using the existing comparator before applying the explicit move.
- [x] Add anchored-create tests: A,B,C -> insert X before B -> A,X,B,C; after B -> A,B,X,C; append afterward is last. Cross-parent/self/missing anchors, top-level targets, unsupported metadata, broken links and nested families refuse without writes.
- [x] Add stale/recovery tests: edits/deletion of any dependency, sibling joins/reparenting and partial journal application. Persist guards for both operations, include the created path in create membership guards, and prove repeated recovery is idempotent. Undo after a remote change refuses.
- [x] Add queue tests: submit while the vdir is locked, observe reordered overlay, submit a second move on that overlay, drain after release, and assert final order plus undo/redo. A stale dependency refuses the queued action with no partial writes; no sync starts merely because an action was queued.
- [x] Add gate tests for dedicated `reorder`/`insert` operation rows. Accept `iphone_baseline = unsupported` only for these Twydo-originated operations, requiring `twydo_to_iphone` and `iphone_to_twydo` passes in verified mode. Missing/unverified evidence refuses. Other operations retain their existing baseline requirements. Validate parent and all affected sibling shapes, metadata and recurrence preservation; the original shape-specific row policy was rejected by the user and is superseded by Task 4’s field-specific gate.
- [x] Run `cargo test -p twydo-core --test store_family --test store_queue --test gate --test journal`; confirm the new tests fail before implementing.
- [x] Implement the operation variants and narrow gate path. Seed plain checklist/project `reorder`/`insert` rows with unsupported phone baseline and unverified round-trip verdicts, documenting the known append representation and the unavailable phone control without claiming verification.
- [x] Implement order planning: sort every sibling, remove/reinsert the moved item or insert the newly created resource, assign checked zero-based ranks and update only differing ranks through `MetaJson` with normal edit timestamps. Hold every unchanged sibling and parent. Set durable family guards for both immediate and queued/recovered paths. Reuse journal and queue protocols; do not add another write path.
- [x] Run the above suites and `cargo test -p twydo-core --test views`; all pass. Update `docs/architecture.md` and `docs/data-model.md` with ordering/queue semantics. Prepare a focused `feat(core): persist sibling display-order changes` commit; hold it if live review is running.

### Task 2: TUI moves and persistent insertion placement

**Files:** modify `crates/tdo/src/app.rs`, `app/newrow.rs`, `form.rs`, `write.rs`, help rendering in `ui/mod.rs`; extend `tests/app_keys.rs`, `tests/write.rs`, `tests/ui_render.rs`; update `docs/tui.md` and `docs/tasks.md`.

**Interfaces:** Consume Task 1's operations and `Placement`. Track pending insertion placement in the new row and its transition into the edit card; the form's create emits `CreateAt` only for an anchored sibling insertion. Extend created-task bookkeeping to recognize both create variants. Continue using `Writer::run` -> `Store::submit` and the existing selection restoration.

- [x] Write failing key tests for `J`/`K` in Tasks on nested checklist/project rows, first/last boundaries, hidden siblings (A,B,C,D with B hidden: A past C -> B,C,A,D), different displayed sections, standalone tasks, detached/flat project rows, Sidebar/Details focus, overlays, search, narrow terminals and selection following the UID after reload.
- [x] Write row/card tests proving `o`/`O` retain before/after placement through Enter and Tab/card save; a deleted/reparented anchor refuses and retains typed input. `a` and subsequent Enter-chain entries still append; top-level creates remain ordinary `Create`.
- [x] Write real-writer tests for both parent types, queued reorders and inserts, undo/redo and no write/undo/sync side effect at a movement boundary. Render Details and Tasks to assert the same resulting sibling sequence.
- [x] Run `cargo test -p tdo --test app_keys --test write --test ui_render`; confirm the relevant failures.
- [x] Implement neighbor selection using visible nested rows in the same parent/section, guard overlays before issuing operations, add concise context/boundary status messages, and wire insertion placement through the card and create bookkeeping. Add help/documentation with enabled/refused contexts and the compatibility gate.
- [x] Run the above suites; all pass. Prepare `feat(tui): reorder checklist and project items`; hold the commit while live review is running.

### Task 3: Full validation and bounded interoperability candidate

**Files:** create `docs/reference/phase0/issue41-item-reorder.md`; update `docs/compatibility.md` and this plan's checkpoint. Only add synthetic changed/new fixtures after a separately approved capture.

**Interfaces:** Consume the tested Twydo-originated operations; the phase-0 document specifies the exact checklist/project actions and resource bounds for approval, not permission to execute them.

- [x] Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py` and `git diff --check`. All must pass. Review the complete branch against the spec and all five Review Focus cases; address material findings with reproducing tests.
- [x] Prepare a bounded phone checklist using synthetic non-recurring undated checklist/project families: Twydo reorder differing from creation order, phone sync/display confirmation, agreed rename/sync return, anchored insertion and its preservation return. Record exact resource identities privately before proposing access; store no credentials/account identifiers in git. Do not ask for phone-side reorder or repeat issue 35 append checks.
- [x] Present the reviewable local candidate and obtain fresh approval for the exact device/server scope before any access/upload. Until approval/evidence, keep matrix round trips unverified and document that verified mode refuses these operations.
- [x] Only if approved, execute the scoped rounds, capture only changed/new synthetic resources, verify ordering/identities/links and unrelated metadata, then update only the actually covered compatibility rows. A mismatch stops the round without expanding scope.
- [x] Record final checks, compatibility limits and review results. Prepare focused docs/evidence commits, respecting any active live review.

### Task 4: Correct the overly broad family-shape gate

User requested this correction after rejecting simple-family-only enablement.

- [x] Replace artificial closed/scheduled/recurrence ordering exclusions with regression tests requiring field-only reorder on existing recurring checklist/project captures, dates/alarms/mixed status, queue and undo.
- [x] Watch those tests fail on the current gate. Prove parent and unrelated file bytes unchanged, all resource fields except DisplayOrder/modification stamps unchanged, and order retained through already-supported recurring completion.
- [x] Validate family structure/JSON/identity once, then check the verified ordering row by declared parent kind, independent of recurrence/date/status. Keep the complete existing Create gate for CreateAt. Do not widen recurrence execution or other operations.
- [x] Update matrix notes/docs to distinguish direct phone sample from field-preservation coverage; keep broken/unsupported structural cases refused. Run targeted tests, workspace, clippy/fmt, Python/links and independent review of this correction.
- [x] Commit the correction and refresh the completed checkpoint. No device/server access or repeat phone actions.

## Checkpoint

Tasks 1–4 complete. The user rejected the simple-family-only gate after the original
phone rounds; Task 4 corrected it to match the DisplayOrder-only write footprint.

Ordering is enabled by declared checklist/project parent kind across recurrence, dates,
alarms and completion state, subject to valid structure/identity/editable JSON and the
verified ordering operation. It preserves all non-order fields. Anchored creation also
passes the complete existing Create gate. No recurrence edit/execute or other operation
verdict was widened. The phone must use **Manually** to show the saved ranks.

Direct phone evidence remains the separately approved plain-family rounds recorded in
`docs/reference/phase0/issue41-item-reorder.md`. Broader field-operation coverage is based
on exact preservation regressions over the existing recurring captures, including queue,
undo, supported completion, mixed status, dates/alarms/opaque recurrence and the normal
verified TUI. No additional device/server access occurred for this correction.

Independent correction review found no issues. All 1,030 Rust tests, 49 Python tests,
clippy with warnings denied, formatting, documentation links and diff whitespace pass.
Branch remains isolated on `issue-41-item-reorder`, based on main `f4db125`.
No additional uploads/phone actions or test-family cleanup are authorized.
