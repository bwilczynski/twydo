# Agent handoff

You are picking up a project whose **design is done and implementation is in progress** (plans in
`docs/superpowers/plans/`). Your job is to continue building Twydo (command `tdo`), a terminal task
app replacing 2Do for one user, plan by plan.

## Read in this order

1. [`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`](docs/superpowers/specs/2026-10-31-terminal-tasks-design.md):
   the spec. It is the source of truth: goals, non-goals, user decisions (§4), proposals (§5),
   data model and compatibility gate (§6), behaviour and notification contract (§7), screen and keys (§8),
   tests (§9), phases (§10), and what's left to you (§11).
2. [`docs/README.md`](docs/README.md): the as-is documentation (how Twydo works today: TUI,
   task behaviour, sync, compatibility, architecture, data model). When you change behaviour,
   update the matching page there in the same change; `python3 tools/check_links.py` must pass.
3. [`docs/reference/2do/`](docs/reference/2do/README.md): consult as needed. It holds exact 2Do encodings,
   e.g. the repeat-rule table (01 §6), date-group thresholds and sort codes (01 §12), and CalDAV/`X-2DOAPP-*`
   metadata (06 §1.7, §2.6). Markers: **[obs]** observed, **[inf]** inferred, **[?]** unresolved.
   If it disagrees with the spec, the spec wins.

## What to do next

1. Read the current plan under `docs/superpowers/plans/` and continue from its first unfinished task.
   Write the next plan (if you use the superpowers workflow: `superpowers:writing-plans`) only when
   the current one is done.
2. **Phase 0 gate: passed and approved (2026-11-01).** See the Result section of
   [`docs/reference/phase0/README.md`](docs/reference/phase0/README.md). Only one 2Do device was available, so many matrix
   rows are still `unverified` with a reason in `compat/matrix.toml`. Before writable work depends on
   one of them (cross-device operations, edge cases, conflicts, unused rule classes), verify it with
   the user's help first (spec §6, §10).
3. Ask the user before any action that touches their production 2Do data, NAS or iPhone.
4. **Done so far:** Plan 1 (read-only foundation), Plan 4 (writable core, `tdo op`), Plan 3
   (sync runner and conflicts, `tdo sync`; verified against the iPhone in
   [`docs/reference/phase0/plan3-sync.md`](docs/reference/phase0/plan3-sync.md)) and Plan 5 (TUI editing: write
   keys, move, edit form, notes in `$EDITOR`; keys in [`docs/tui.md`](docs/tui.md), verified in
   [`docs/reference/phase0/plan5-tui.md`](docs/reference/phase0/plan5-tui.md)) and Plan 6 (checklists, projects,
   recurring projects and the real repeat shapes, mirroring 2Do's family semantics; verified in
   [`docs/reference/phase0/plan6-baseline.md`](docs/reference/phase0/plan6-baseline.md)), and the gap session
   that verified the real task shapes both ways
   ([`docs/reference/phase0/gap-session.md`](docs/reference/phase0/gap-session.md); seed test data with
   `tools/seed_fixture.py` instead of typing on the phone), and the public-repository docs
   (`docs/superpowers/plans/2026-11-02-public-repo-docs.md`: README, `docs/` pages, MIT licence,
   VHS demo from `tools/demo_vdir.py`), and issue #10 (a save blocked by a stuck sync offers to
   stop it; `docs/superpowers/plans/2026-11-03-issue-10-stop-stuck-sync.md`), and the release
   pipeline (`docs/superpowers/plans/2026-11-03-release-pipeline.md`: CI on PRs, release-plz
   release PRs from conventional commits, cargo-dist binaries, `mise use github:bwilczynski/twydo`),
   and Plan 7 phase 1 (adding and reshaping tasks: inline add with `o`/`O`/`a`, the in-place edit
   card, type changes `ct`/`cp`/`cc`, all repeat kinds; plan
   `docs/superpowers/plans/2026-11-03-plan-7-add-and-reshape.md`, design
   `docs/superpowers/specs/2026-11-03-tui-add-and-structure-design.md`, iPhone capture in
   [`docs/reference/phase0/plan7-capture.md`](docs/reference/phase0/plan7-capture.md)). Task 13, the phone
   verification of Twydo's own writes, is done (session B, "Results B" in the capture page): verified
   mode now creates checklists, projects, recurring projects and items, and creates every repeat
   kind except the three 2Do 3.91 misreads (last day, last weekday, Nth weekend day; rule `edit` =
   `fail`, never written). Type changes are allowed (task to checklist/project, checklist to
   project, empty parent to task); setting/removing/changing weekly and monthly repeats is allowed; daily, yearly,
   from-completion and project/checklist repeat changes stay refused (their `repeat` rows lack
   verdicts). Tasks 14 and 15 are done too (session C, "Results C" in the capture page): Twydo
   completes every repeat kind it creates exactly as the phone does (twin byte-compares in
   `crates/twydo-core/tests/edit_complete.rs`), so verified mode completes them all but an end
   date (`mod:until` `execute` stays unverified), and edits existing daily and weekly tasks
   (weekly to monthly and weekly removal included). A last-weekday rule is written with 2Do's
   own `RecurrenceValue` 6 (-1 still read). Refused on purpose: completing a recurring project
   that ends or repeats from completion, and changing an Nth-weekday repeat's interval.
   **Follow-ups:** an "until" series that goes on (the phone's completion and history copy);
   the remaining `repeat` rows (daily, yearly, from-completion, projects, checklists); editing
   existing from-completion tasks; empty project to checklist; what 2Do does when an ended
   series' last occurrence is reopened (Twydo refuses, undo works); recurring projects that end
   or repeat from completion; the unused edge cases `monthly-31st`, `yearly-feb-29`,
   `from-completion-anchor`, `end-after-n`, `end-on-date` could cite session C.
   Next on the roadmap: those follow-ups as needed, then the design's later features (indent and
   outdent, yank and paste, list management, Project View; these fold in most of Plan 5b),
   natural-language input on the new row (phase 3, reusing `twydo_core::dateinput` and the repeat
   parser), manual order `J`/`K`, or Plan 2 (notifier). Write the plan first.

## Ground rules

- **No personal data in this repository.** Never commit real task titles, list names, notes, dates of
  real obligations, people's names, addresses, account identifiers, credentials, CalDAV URLs/usernames,
  or raw exports (Dropbox `2Do/` folder, decoded JSON, `.ics` from the user's production calendars).
  Test fixtures must come from the **isolated test account** with synthetic content, or be anonymised
  before committing. `.gitignore` blocks the common cases; check `git diff --staged` anyway.
- The spec refers to acceptance cases **P1, P2, L1** by label. Their real names live outside the repo in
  `../rea-private/acceptance-cases.md` (on the user's machine only). You may read it locally for
  acceptance testing; never copy its contents into the repo.
- Code lives in this repository as a Cargo workspace at the root:
  - `crates/twydo-core/`: core library (data model, storage, views)
  - `crates/tdo/`: the `tdo` binary (CLI and TUI)
  - `compat/`: the phase-0 compatibility matrix (`matrix.toml`)
  - `contrib/`: example configuration for external tools (vdirsyncer, systemd, launchd)
  - `tools/`: helper scripts (tests in `tools/tests/`)
  - `docs/`: the as-is pages at its root, `docs/demo/` (VHS demo), `docs/reference/` (2Do reference
    and the phase-0 checklist and session logs), `docs/superpowers/` (spec and plans)
- The spec's "decisions made by the user" (§4) are fixed; "decisions made by me" (§5) are proposals you
  may refine with justification, except where §11 says they cannot be overridden (compatibility gate,
  preservation, conflict, recovery and notification contracts).
