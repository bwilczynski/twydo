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
   [`docs/reference/phase0/plan7-capture.md`](docs/reference/phase0/plan7-capture.md)).
   **Current follow-up:** Plan 8's [resume checkpoint](docs/superpowers/plans/2026-11-04-plan-8-editing-completeness.md)
   and [session coverage ledger](docs/reference/phase0/plan8-tui-completeness-session.md) record
   completed evidence and remaining work across all existing TUI actions. Batch 1 and RF/RA
   phone rounds are complete; do not repeat them. See [compatibility](docs/compatibility.md)
   for exact enabled and refused shapes, including date-only daily/plain-monthly until execution.
   Prepare a bounded checklist and obtain fresh approval before additional device rounds. Private
   session state and backups are outside git. Do not touch other data, change accounts or global
   write mode, or delete the synthetic test lists without the agreed consent.
   Next on the roadmap: those follow-ups as needed, then the design's later features (indent and
   outdent, yank and paste, list management, Project View; these fold in most of Plan 5b),
   natural-language input on the new row (phase 3, reusing `twydo_core::dateinput` and the repeat
   parser), the write queue in `twydo-core` (edits during a sync are
   queued instead of refused; design
   [`docs/superpowers/specs/2026-11-06-write-queue-design.md`](docs/superpowers/specs/2026-11-06-write-queue-design.md),
   issue #38; write-queue plan 1, batched sync with `tdo sync on|off`, merged (PR #50); plan 2
   (the queue with strict whole-file checks) done
   (`docs/superpowers/plans/2026-11-06-write-queue-2-queue.md`). **Decided 2026-11-09:**
   `tdo` gets a daemon that owns the data, sync and scheduling; the TUI and CLI become
   clients over a Unix socket
   ([daemon design](docs/superpowers/specs/2026-11-09-daemon-design.md)). Build it before write-queue plan 3 (rebasing, now inside the
   daemon) and before Plan 2 (the notifier, which lives in the daemon); do not extend the
   cross-process queue, waiter or sync timer meanwhile. **Daemon plan 1 is merged** (PR #78;
   `docs/superpowers/plans/2026-11-09-daemon-1-core-and-writes.md`). **Daemon plan 2 is done**
   on `feat/daemon-2-sync-jobs` (`docs/superpowers/plans/2026-11-10-daemon-2-sync-and-jobs.md`):
   the hidden `tdo daemon` runs syncs and list jobs on its vdir lane with durable job records,
   `job.get` and `sync.started`/`sync.finished`/`job.finished` events; serves `sync.run`,
   `stop` and `set_auto` with a scheduler (startup, every 60 s, batched, after a recorded
   conflict choice, switch turned on, wake from sleep); reaps an orphaned vdirsyncer at startup
   and shuts down cleanly during a job (`shutdown_grace`, `when_idle`); serves
   `conflicts.list`/`resolve` (replayable), `journal.list`/`recover`/`revert`/`abandon`, every
   `lists.*` action as a job (forget and recover included; recover is undoable per session),
   `input.parse`, and live `config.toml` reload (binding fields need a restart). No client uses
   it yet: the TUI, `tdo sync`, the waiter and the timer still run their own syncs. Next: plan 3
   (the TUI and CLI switch to the daemon; the Writer and the batch waiter go away; plan 1's
   deferred client-side items are in that plan's notes). Spec decision D13: once clients switch, only the daemon
   reads `config.toml`. The default socket is per user; other setups choose theirs with
   `--socket` or `TWYDO_SOCKET` (the test setup will need `TWYDO_SOCKET` next to
   `XDG_CONFIG_HOME` once plan 3 lands).

## Issue 41 checkpoint

Checklist/project item reordering (`J`/`K`) and saved `o`/`O` placement are implemented;
see `docs/superpowers/plans/2026-11-06-issue41-item-reorder.md` and the session record in
`docs/reference/phase0/issue41-item-reorder.md`. Ordering is a DisplayOrder-only operation
for recurring and non-recurring families; it preserves dates, recurrence, alarms and
completion state rather than requiring unrelated operation verdicts. Anchored creation
also passes the existing Create gate. The phone must select **Manually** to display the
saved ranks. Both bounded phone rounds passed; direct samples were plain families and
recurring-family coverage uses exact preservation tests over existing captures. Do not
repeat uploads/phone actions or delete the synthetic families. Additional device scope
requires fresh approval.

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
