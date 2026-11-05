# Issue 35 item order implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Preserve checklist/project item sequence and append new items reliably.

**Architecture:** Parse ordering metadata once, use it only for nested sibling ordering,
and assign append positions in the store with sibling snapshot guards. Keep the Enter
chain at the end of the visible family. Reuse captured evidence without duplicating files.

**Tech Stack:** Rust, existing Cargo workspace, ratatui; existing Python fixture tools.

**Spec:** [Approved design](../specs/2026-11-05-issue35-checklist-order-design.md).

## Global constraints

- Top-level/date-view sorting and project-item scheduling remain unchanged.
- Preserve unknown metadata, existing sibling bytes and timestamps; no new dependencies.
- No device/server writes until the candidate is reviewable and its exact scope approved.
- Evidence stores only new/changed resources and references unchanged captures.
- Focused commits, no agent attribution. Live diff declined by the user.

## Review focus

1. Equal/missing creation times and malformed ordering values: deterministic ordering.
2. Sparse/overflowing positions and completed siblings: safe append, no renumbering.
3. A sibling changed/joined after load: refuse without partial writes, including recovery.
4. Independently eligible project children: ordinary flat-view date sorting persists.
5. Enter chains and family completion/copies: stable item order and preserved metadata.

### Task 1: Read and display sibling order

**Files:** `crates/twydo-core/src/model/meta.rs`, `crates/twydo-core/src/views.rs`,
`crates/twydo-core/tests/views.rs`, `docs/tasks.md`, `docs/data-model.md`.

**Interfaces:** Produce `Metadata::display_order() -> i64` (nonnegative, defaults to zero)
and map JSON `DisplayOrder` to token `displayorder`. Use a nested sibling comparator in
both tree and flat checklist paths; preserve the ordinary `compare` for other rows.

- [x] Add captured-family view regressions for checklist/project creation and append,
  reconstructing the project delta from its baseline. Assert Zulu, Alpha, Mike, Bravo.
  Cover tree/list sort overrides and flat checklist views, equal/missing timestamps,
  malformed/absent positions, and independent project-child date sorting.
- [x] Run `cargo test -p twydo-core --test views`; confirm the new regressions fail.
- [x] Implement metadata parsing and nested ordering by position, creation, UID; update docs.
- [x] Run the same view suite; expect all tests pass. Commit the reader fix.

### Task 2: Append safely and keep the Enter row at the end

**Files:** `crates/twydo-core/src/store/mod.rs`, `crates/twydo-core/src/store/family.rs`,
`crates/tdo/src/app/newrow.rs`, `crates/twydo-core/tests/store_family.rs`,
`crates/tdo/tests/write.rs`, `crates/tdo/tests/app_keys.rs`, `docs/tui.md`,
`docs/architecture.md`, `docs/data-model.md`.

**Interfaces:** Consume `Metadata::display_order()` and existing `MetaJson`, journal holds
and family guards. New child position is `max(sibling_count, max_position + 1)`, or zero
when empty; checked arithmetic refuses overflow. Persist sibling holds and membership
guards for create; include the created path in durable guards for idempotent recovery.

- [x] Add real-store tests for both parent types: captured append position, same-second
  appends, empty/sparse/completed/malformed/overflow siblings, unchanged neighbor bytes,
  stale sibling edits/joins, recovery after a create, undo/redo and family copy retention.
  Extend Enter-chain tests to assert the next row is at the visible family's end.
- [x] Run `cargo test -p twydo-core --test store_family` and
  `cargo test -p tdo --test write --test app_keys`; confirm relevant new failures.
- [x] Implement append metadata and guards, reuse end-of-items anchoring in the Enter chain,
  and update docs. Do not change standalone creation or rewrite siblings.
- [x] Run the same suites, then `mise run test`, `mise run lint`, `git diff --check`.
  Expect all checks pass; review the full branch with a fresh reviewer. Commit focused fixes.

### Task 3: Bounded phone return (approval gate)

**Files:** existing issue35 capture notes, new/changed synthetic fixtures only,
`docs/compatibility.md` if the check succeeds.

**Interfaces:** The tested store creates one `Echo` under each existing synthetic parent.

- [x] Prepare the two exact candidate resources and a fresh read of their family dependencies
  privately, without uploading. Present exact scope for fresh approval.
- [ ] Only after approval, upload the two new resources with creation preconditions; ask the
  user to inspect end placement and rename each Echo to ECHO, then capture the return.
- [ ] Verify identities/links/order and unchanged siblings; add return assertions in existing
  suites, storing only changed/new files. Run relevant checks and commit evidence/docs.
  If blocked or mismatched, record the precise limitation; do not claim interoperability.

## Execution note

The user approved the design and instructed continued implementation in this session.
Tasks 1–2 execute inline; Task 3 stops before uploads for the explicitly required approval.

## Local checkpoint

Tasks 1–2 are implemented in focused commits; final 865 Rust tests, 49 Python tests,
links, clippy and formatting pass. Independent review completed; its Important Details
ordering finding was reproduced and fixed with a rendering regression. The legacy UID-tie
minor is documented and deferred. Task 3 candidates are
prepared privately (two Echo resources, position 4); no uploads are authorized yet.
