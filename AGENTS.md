# Agent handoff

You are picking up a project whose **design is done and implementation is in progress** (plans in
`docs/superpowers/plans/`). Your job is to continue building Twydo (command `tdo`), a terminal task
app replacing 2Do for one user, plan by plan.

## Read in this order

1. [`docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`](docs/superpowers/specs/2026-10-31-terminal-tasks-design.md):
   the spec. It is the source of truth: goals, non-goals, user decisions (§4), proposals (§5),
   data model and compatibility gate (§6), behaviour and notification contract (§7), screen and keys (§8),
   tests (§9), phases (§10), and what's left to you (§11).
2. [`docs/design/tui-layout.md`](docs/design/tui-layout.md): the chosen layout.
3. [`docs/reference/2do/`](docs/reference/2do/README.md): consult as needed. It holds exact 2Do encodings,
   e.g. the repeat-rule table (01 §6), date-group thresholds and sort codes (01 §12), and CalDAV/`X-2DOAPP-*`
   metadata (06 §1.7, §2.6). Markers: **[obs]** observed, **[inf]** inferred, **[?]** unresolved.
   If it disagrees with the spec, the spec wins.

## What to do next

1. Read the current plan under `docs/superpowers/plans/` and continue from its first unfinished task.
   Write the next plan (if you use the superpowers workflow: `superpowers:writing-plans`) only when
   the current one is done.
2. **Phase 0 gate: passed and approved (2026-11-01).** See the Result section of
   [`docs/phase0/README.md`](docs/phase0/README.md). Only one 2Do device was available, so many matrix
   rows are still `unverified` with a reason in `compat/matrix.toml`. Before writable work depends on
   one of them (cross-device operations, edge cases, conflicts, unused rule classes), verify it with
   the user's help first (spec §6, §10).
3. Ask the user before any action that touches their production 2Do data, NAS or iPhone.
4. **Done so far:** Plan 1 (read-only foundation), Plan 4 (writable core, `tdo op`), Plan 3
   (sync runner and conflicts, `tdo sync`; verified against the iPhone in
   [`docs/phase0/plan3-sync.md`](docs/phase0/plan3-sync.md)) and Plan 5 (TUI editing: write
   keys, move, edit form, notes in `$EDITOR`; keys in [`docs/tui.md`](docs/tui.md), verified in
   [`docs/phase0/plan5-tui.md`](docs/phase0/plan5-tui.md)) and Plan 6 (checklists, projects,
   recurring projects and the real repeat shapes, mirroring 2Do's family semantics; verified in
   [`docs/phase0/plan6-baseline.md`](docs/phase0/plan6-baseline.md); open: checklist items'
   own due dates when the checklist's date changes). Next: natural-language add (`o`/`O`,
   reusing `twydo_core::dateinput`), Plan 5b (manual order `J`/`K`, list management) or Plan 2
   (notifier); write the plan first. Setup and limits of sync: [`docs/sync.md`](docs/sync.md).

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
  - `contrib/`: example configuration for external tools (vdirsyncer)
  - `tools/`: helper scripts; `docs/`: spec, design, 2Do reference, plans, config, and the
    phase-0 checklists (`docs/phase0/`)
- The spec's "decisions made by the user" (§4) are fixed; "decisions made by me" (§5) are proposals you
  may refine with justification, except where §11 says they cannot be overridden (compatibility gate,
  preservation, conflict, recovery and notification contracts).
