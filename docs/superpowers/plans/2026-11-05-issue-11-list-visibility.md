# Issue #11 list visibility implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Always show future-start and completed work in All and ordinary lists.

**Architecture:** Extend pure view membership and grouping, preserving the existing
build_view/counts APIs. Split tree nesting only at ordinary/future-start/completed section
boundaries; reuse the TUI's existing view rendering and UID selection restoration.

**Tech Stack:** Rust, ratatui, existing Cargo workspace.

**Spec:** [Approved scope](../specs/2026-11-05-issue-11-list-visibility-design.md).

## Global constraints

- No new shortcuts, status hints, visibility preferences, configuration or persistence.
- No task-resource writes or production/device round.
- Today, Scheduled, Done and Next 3 Days retain their current rules.

## Review focus

- Completed children under collapsed open parents remain discoverable in Done.
- Future-start tasks with overdue due dates still belong to Future.
- Date buckets do not detach ordinary active children from expanded parents.
- All exclusions still apply to newly visible rows; direct list access remains possible.
- Completed rows never inflate unfinished totals, including children and dated history.

## Task 1: core membership, sections and counts

Files: `crates/twydo-core/src/views.rs`, `crates/twydo-core/tests/views.rs`.
Interfaces: retain `build_view`, `counts`, `ViewResult`; add `DateGroup::Done`.

- [x] Add tests for default All/list inclusion, Future/Done precedence and completion ordering,
  non-date sorts, empty sections, family section boundaries, counts and collection exclusions.
- [x] Run `cargo test -p twydo-core --test views`; confirm new assertions fail on old behavior.
- [x] Extend eligible membership to open/completed in All/lists, restrict unfinished counts,
  classify section boundaries independently of date buckets, sort Done by completion time,
  and add its group after ordinary/Future rows. Preserve item ordering within each section.
- [x] Update earlier expectations that intentionally encode the superseded hiding contract.
- [x] Run `cargo test -p twydo-core --test views`; expect all tests pass.

## Task 2: TUI and documentation

Files: `crates/tdo/src/ui/tasks.rs`, `crates/tdo/tests/ui_render.rs`,
`crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/write.rs`,
`crates/tdo/src/app/newrow.rs`, `docs/tasks.md`, `docs/tui.md`,
`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`.
Interfaces: render `DateGroup::Done` as DONE using existing group rendering; retain App APIs.

- [x] Keep detached child parent hints visible and anchor new items within the parent's
  own section; verify the existing real-store Enter chain with a completed sibling.
- [x] Add rendering tests for Future/Done, omitted empty sections, and unchanged special views;
  add selection tests for completion/reopen/reload with the newly retained completed rows.
- [x] Run the new TUI tests and confirm missing behavior fails before implementation.
- [x] Add the DONE label and update stale expected list rows/counts without changing controls.
- [x] Update matching documentation and the original spec with the approved behavior change.
- [x] Run `mise run lint`, `mise run test` and `git diff --check`; expect all pass.
- [x] Review the complete diff, offer final terminal review, and leave changes reviewable
  without committing while a working-tree diff review may be pending.

## Result

Implemented in `feat/issue-11-list-filtering`. The user declined terminal diff review
and requested focused commits and a pull request.
All/list Future and Done sections are always on; special views retain their rules.
The review found and fixed detached-child parent hints and item-row anchoring in families
with completed children. A regression also pins shared due ordering within Future.

Validation: `mise run lint`, `mise run test`, `python3 tools/check_links.py` and
`git diff --check` passed. No production/device data or sync operations were used.

## Follow-up: fixed 30-day Done sections

User request: show the last 30 days in All and ordinary lists; keep the dedicated Done view's
full history. Use 30 local calendar days including today, without deleting any task resources.
Unknown completion dates remain available in the dedicated Done view.

- [x] Add failing boundary/expiry tests spanning DST, unknown/future dates, and a recent child
  of an expired completed parent in `crates/twydo-core/tests/views.rs`.
- [x] Filter completed membership in All/lists by local completion date; leave `View::Done`
  unrestricted. Give existing synthetic completed-child fixtures explicit completion dates.
- [x] Update the as-is docs and design; run lint, full tests and links.
- [x] Commit this follow-up and update the existing PR.

Follow-up verification: core view tests 33/33; full Rust and Python suites, lint and
documentation links passed. Read-only review found no actionable defects.

## PR #47 review correction: reopened child selection

- [x] Reproduce lost selection when reopening a detached completed child beneath a
  collapsed parent in All and ordinary lists.
- [x] Expand the reopened child's ancestors before restoring selection; unchanged
  reloads leave collapsed families alone. Document the behavior in `docs/tui.md`.
- [x] Verify regression coverage for project and checklist parents in both views,
  full lint/tests and documentation links, and obtain an independent code review.
