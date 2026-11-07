# Capability gating 1: truthful reporting — Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for inline execution or superpowers:subagent-driven-development if requested. Track steps with checkboxes.

**Goal:** Make `tdo check` and `tdo op check` describe actual operation permissions without changing writer admission.

**Architecture:** Share typed permission probes in the CLI layer, calling the existing core gate. Both reports use configured write mode and the queued snapshot. Keep recurrence evidence separate from data health and operation availability.

**Tech Stack:** Existing Rust workspace, chrono, clap and core gate/queue APIs; no new dependencies.

**Spec:** [Approved capability-gating design](../specs/2026-11-07-capability-gating-design.md).

## Global constraints

- Work in the user-selected worktree; all file paths in this plan are repository-relative.
- This increment does not widen permissions, modify matrix verdicts or change core writers.
- “Unchanged scheduling fields do not require recurrence execution/edit verdicts” is the approved migration policy; subsequent increments implement its capability paths.
- “Every additional phone case must name an unresolved failure mechanism and explain why existing captures or a local test cannot answer it.” No device/server work is needed here.
- “Exit codes: 0 means the report was produced; 2 means invocation/configuration/read failure.” Partial parse failures remain report content as today.
- “Summaries group refusals by action and reason; task titles and paths stay behind --verbose.” Do not print raw parse messages containing paths by default.
- Preserve recovery/locking and queued-overlay behavior. A report is gate admission at a snapshot, not a promise that later store preconditions will pass.
- Keep changes uncommitted during live review; no agent attribution. No production data or personal content in tests/docs.

## Review focus

1. Passing rule-level verdicts must not hide refused edits/deletions (Task 2).
2. Completed, cancelled, duplicate-UID and history resources must not get misleading lifecycle availability (Tasks 1–2).
3. Queued creations/deletions must affect both reports, and reporting must not drain the queue or start sync (Task 3).
4. Default output must not leak task titles, notes or parse-error paths, including new refusal summaries (Task 2).
5. Parameter-dependent actions and mixed write modes must not be advertised as blanket permission (Tasks 1–3).

## Files and boundaries

- Create `crates/tdo/src/cli/permissions.rs`: shared typed probes, per-task output and summary helpers; no new gate policy.
- Modify `crates/tdo/src/cli/mod.rs`: register module and add a shared queued-snapshot loader.
- Modify `crates/tdo/src/cli/check.rs`: data health, recurrence evidence and aggregated permissions.
- Modify `crates/tdo/src/cli/op.rs`: delegate check output and snapshot loading without changing write submission.
- Modify `crates/tdo/src/main.rs`: accurate check command help.
- Extend `crates/tdo/tests/cli.rs` and `crates/tdo/tests/cli_op.rs`: process-level output/config/queue regressions.
- Update `docs/compatibility.md`, `docs/config.md`, `docs/architecture.md`, main spec and this checkpoint.

### Task 1: Shared concrete permission probes

**Interfaces:** In `cli/permissions.rs`, add `pub(super) enum Availability { Allowed, Refused(String), Inapplicable(&'static str), RequiresInput(&'static str) }`, `pub(super) struct Probe { pub action: &'static str, pub availability: Availability }`, and `pub(super) fn probes(snap: &Snapshot, matrix: &Matrix, mode: WriteMode, idx: usize, now: DateTime<Utc>) -> Vec<Probe>`. Add `pub(super) fn task_lines(probes: &[Probe]) -> Vec<String>` for both commands.

- [x] Add failing module tests: title and notes map to separate `Op::Edit` values; compare their results with direct `gate::check` calls. Use a summary edit with the existing title and a description edit with the existing optional description; no writes occur.
- [x] Pin lifecycle states: open tasks probe complete; completed tasks probe reopen; the opposite actions are inapplicable. Cancelled tasks have both lifecycle actions inapplicable pending an explicit cancellation policy. Title, notes and delete still invoke the gate for each status.
- [x] Pin parameterized output: `create`, `edit dates`, `edit repeat`, `convert`, `move`, `reorder`, `insert` report required input/context, with no invented successful shape or chosen destination. Give each distinct label and short explanation. Include a reminder that this is a snapshot of admission.
- [x] Run `cargo test -p tdo --lib cli::permissions`; expect missing/new behavior failures before implementation.
- [x] Implement probes using only `gate::check`, fixed `now` converted through snapshot timezone, and the above status applicability. For duplicate identities, gate lookup must refuse; do not report permission for the first match alone.
- [x] Run the module tests; require all pass. Keep the new module private to CLI.

### Task 2: Replace the misleading global pass/fail report

**Interfaces:** `check::run` keeps its existing signature. Consume Task 1 probes. Retain existing per-rule evidence lines, but remove all `GATE: PASSED` / `GATE: NOT PASSED` output.

- [ ] Replace old global-gate assertions in `tests/cli.rs` with failures requiring separate `Data health`, `Recurrence evidence`, and `Operation availability` output. Pin `parse errors` and `model read-only resources` counts; include blocked completed resources, not just open resources.
- [ ] Reproduce the misleading success: a synthetic daily task with passing rule rows but missing operation rows reports healthy input and refused title/delete actions, with reasons. Alter only the rule edit verdict to unverified: retain that evidence and derive permissions from actual gate results rather than inventing a global failure.
- [ ] Add empty-vdir and malformed-file assertions, completed/history lifecycle reporting, duplicate identities and cancelled applicability. Counts are per task/action probe, not unique UID lookup; summaries distinguish allowed/refused/inapplicable/needs-input and group refusals by action/reason.
- [ ] Pin default privacy and --verbose expansion with synthetic titles/notes/paths; notes are never needed in either output. A missing explicit --matrix exits 2 with a load error. Successful reports containing refusals or per-file parse errors exit 0. Invalid config/unreadable vdir exits 2.
- [ ] Run `cargo test -p tdo --test cli gate::`; verify expected new-output failures.
- [ ] Implement the report, using `Matrix::load` for any explicit matrix. Display configured write mode and describe rule verdicts as evidence, not permissions. Include task count and explicit empty-input message. Use stable action/reason ordering and verbose titles only beneath their relevant groups.
- [ ] Update command help and `docs/compatibility.md` with actual report meaning, exit convention, partial-input caveat and parameterized-action limitations. Do not claim generic edit/create/move permission.
- [ ] Run `cargo test -p tdo --test cli gate::`; require all pass.

### Task 3: Align per-task checks and queued data

**Interfaces:** Add `pub(super) fn permission_snapshot(cfg: &Config) -> Result<Snapshot, CliError>` in `cli/mod.rs`: load the vdir through the existing reader lock, release it as existing op snapshot code does, then apply `queue::view(report, cfg.clone())`. Keep startup recovery behavior in existing callers. Both reports use this helper; export/TUI loaders retain their current behavior.

- [ ] Update `op_check_lists_every_kind` to assert the concrete Task 1 action labels and distinct lifecycle/input states. Add verified/test mode comparisons against direct gate decisions with a custom baseline-only matrix. Missing UID keeps exit 1.
- [ ] Add a fixture-backed comparison: per-task title/notes/delete refusals match the corresponding aggregate groups from `tdo check`, including recurring-checklist or from-completion text restrictions. Do not change those restrictions in this increment.
- [ ] Add pending-queue regression with temporary vdir/state/config: enqueue a creation or deletion while the vdir is locked, release it without draining, then verify both commands see the same overlay. Assert queue/task bytes remain unchanged by reporting and no sync process is launched. Reuse existing queue helpers, not phone data.
- [ ] Preserve the existing reader-lock test and no-sync check regression. Isolate both XDG_CONFIG_HOME and XDG_STATE_HOME in new command fixtures.
- [ ] Run `cargo test -p tdo --test cli_op`; verify new output/overlay tests fail before integration.
- [ ] Replace `check_lines` with Task 1 helpers; use `permission_snapshot` in both check paths. Preserve op command Store startup/recovery and all non-reporting operation behavior. Map helper errors through existing Failure/CliError conversions.
- [ ] Run `cargo test -p tdo --test cli --test cli_op`; require all pass. Update architecture/config docs for the shared permission report and overlay.

### Task 4: Record policy and validate the increment

- [ ] Amend main spec §5–6 to describe evidence for an operation's actual write contract, using representative interoperability plus preservation reasoning and distinguishing it from direct phone coverage. Keep targeted recurrence execution requirements. Link the approved design as authority; update §9 so no global check result implies all operations are enabled.
- [ ] Record follow-ups: text-edit audit first, then deletion/moves, using specific blocker/write set/reusable evidence/missing proof/boundary. Exact-operation dry run is a follow-up. Do not create the next implementation plan or widen gates in this increment.
- [ ] Run `cargo test --workspace`, `python3 -m unittest discover -s tools/tests -t .`, `python3 tools/check_links.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`. A fresh worktree may need bootstrap/build artifacts; reuse a suitable disk-backed target directory if necessary.
- [ ] Review the whole change against the five Review Focus entries; fix any material findings and rerun affected checks. Confirm no core gate/store writer or compatibility-verdict changes slipped in. Record results here.
- [ ] Offer the required final diff review unless a live review is running. Hold commits while the user reviews. Suggested later commit: `fix(cli): report effective operation permissions` with explicit paths and no attribution.

## Checkpoint

Design approved. Plan prepared; implementation has not started. This plan covers
only truthful reporting and policy documentation. Feature unblocking follows as
separate increments under the approved capability contracts.
