# Issue 42: Manual List Retirement and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Identify missing lists on sync failure, accept their remote deletion explicitly, and recover preserved local tasks into a named existing list.

**Architecture:** Rust owns backups, journal integration, and TUI commands. A confirmed retirement preserves data before removing the collection and sets a durable discovery request; the existing runner executes ordinary vdirsyncer discover before its next sync. Recovery uses the existing gated move planner on an archived snapshot and commits only destination writes.

**Tech Stack:** Existing Rust workspace, serde JSON, durable filesystem primitives, vdirsyncer subprocesses, ratatui.

**Spec:** [approved design](../specs/2026-11-05-issue-42-deleted-list-recovery-design.md)

## Global Constraints

- No production NAS, phone, account or data access is authorized.
- No Python helper or automatic deletion detection.
- Never change compatibility verdicts, overwrite destination tasks, or automatically answer yes to creation prompts.
- Backups survive recovery and normal retention. Preserve pending journal and conflict evidence.

## Review Focus

- A generic item-level Not Found must produce guidance rather than automatic retirement.
- An interrupted retirement must resume before journal recovery or sync can recreate the source.
- A pending operation spanning lists must retain the surviving list's writes.
- Recovery must preserve family identities and reject the whole operation when any source is unsupported.
- A successful recovery followed by a crash/retry or undo must not duplicate or overwrite tasks.

### Task 1: Sync guidance
Files: crates/tdo/src/cli/sync.rs, crates/tdo/src/term.rs, crates/tdo/tests/sync_e2e.rs.
- [ ] Write failing tests that errors name the affected list/id and include :forget-list, including multiple failures.
- [ ] Add reusable missing-list guidance to per-list reporting and retain all stderr lines in the TUI result.
- [ ] Verify targeted tests; retain ordinary errors for authentication/offline failures.

### Task 2: Durable list retirement and rediscovery
Files: new crates/twydo-core/src/retired.rs, src/lib.rs; store/journal.rs, store/mod.rs; sync/run.rs, sync/resolve.rs, sync/mod.rs.
Interfaces: retired::list(cfg), retired::get(cfg,id), retired::forget(cfg,collection,expected,now); retired::resume_locked(vdir,state); sync runner consumes sync/rediscover.
- [ ] Write failing synthetic tests for complete backups, pending edits, spanning journals, conflicts, symlinks, stale snapshots, retry/interruption.
- [ ] Publish backup manifest and exact files/evidence before cleanup. Resume manifest-backed cleanup under the exclusive lock before ordinary journal recovery.
- [ ] Exclude archived journal paths from active work without marking them synced; retain active parts and block undo/resolution to archived paths.
- [ ] Under the run lock, run standard discover with no automatic answers when rediscovery is pending; retain the request on failure, clear only on success.
- [ ] Verify filesystem-pair rediscovery, failure/retry, and empty collection sets.

### Task 3: Journalled recovery
Files: store/retired.rs (new), retired.rs, tests/retired.rs.
Interfaces: Store::recover_list(active_snapshot,backup_id,destination,now) returns journal operation id; retired::snapshot supplies a virtual original-path snapshot.
- [ ] Write failing tests for gated whole-set recovery, family links, collisions, unsupported source, repeated recovery, interruption and undo/redo.
- [ ] Plan existing Op::Move for each archived root, retain destination writes only, validate the full plan, record the stable recovery entry before applying it.
- [ ] Retain archive and operation identity mapping; refuse a different destination or changed files on retry. Use normal session undo/redo stacks.
- [ ] Verify core retirement/recovery tests and existing store/sync tests.

### Task 4: TUI commands and documentation
Files: app/recovery.rs and ui/recovery.rs (new); app.rs, term.rs, write.rs, ui/mod.rs, docs/tui.md, docs/sync.md, docs/architecture.md.
- [ ] Write failing command tests for :forget-list confirmation/cancellation, :recover browsing/preview/destination arguments with spaces, unknown/ambiguous targets, and confirmed writes.
- [ ] Add a focused recovery overlay and confirmation state. Route recovery through Writer; route retirement through the sync worker and reload afterward.
- [ ] Update matching docs with command examples, backup location, preserved local snapshot, and failed-discovery handling.
- [ ] Run mise run test, mise run lint, git diff --check; request a fresh final review and fix material findings.
- [ ] Offer completed diff review; hold commits while review is pending.

## Execution record

Baseline: cargo test --workspace passed in the isolated worktree; sync parser/runner 81 tests passed.
Execution: inline in the existing session, as authorized by "confirmed" and "proceed without a live viewer".
