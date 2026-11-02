# Public repository and documentation: design

Date: 2026-11-02. Step 2 of the roadmap to production (after the gap session, before issue #10
and the release pipeline).

## Intent

The GitHub repository `bwilczynski/twydo` becomes public. The people who will find it are
**current 2Do users looking for a free terminal alternative** that keeps working with 2Do on their
iPhone. The README must show them what Twydo is and how far along it is. Readers who want more
follow links to documentation that explains the mechanics: sync, the data model, the architecture,
and how tasks behave (how a monthly repeat moves, what the difference between a project and a
checklist is, what completing a project does in practice). That knowledge was gained by reverse
engineering and by testing against the iPhone, but it is written as "this is how it works", not
as a log of how it was found.

Today the documentation is spread across `README.md`, `AGENTS.md`, `docs/tui.md`,
`docs/design/tui-layout.md`, `docs/sync.md`, `docs/config.md`, `docs/journal.md` and the
phase-0 session logs. Several of these describe the state at the time they were written.

## Goals

1. A README that works as a front page: what Twydo is, a demo, what works, what does not yet
   work (stated plainly, with where it is headed), how it coexists with 2Do on the iPhone, how to
   install and start, and links into `docs/`.
2. "As-is" documentation at the root of `docs/`, one page per topic, rewritten from the raw
   material so that it describes the current behaviour in one consistent place.
3. Reference material kept and lightly organised, **not rewritten**.
4. Decompilation details removed from the current tree (see below).
5. An MIT licence.
6. A demo of the TUI recorded from **synthetic data only**.

## Non-goals

- Rewriting git history. The decompilation details stay in older commits; the user accepted
  this (2026-11-02). No `git filter-repo`, no force-push, no repository re-creation.
- The release pipeline, mise installation and issue #10 (later roadmap steps). Install
  instructions describe what works today.
- Rewriting the reference material, the spec or the plans.

## Anti-goals (a failure even if everything else is done)

- Any personal data in the repository or in the demo: no real task titles, list names, dates of
  real obligations, people's names, CalDAV URLs or account details (AGENTS.md ground rules). The
  demo and the screenshot use made-up data, never the user's own.
- The README overselling: a 2Do user must not come away thinking Twydo is a finished
  replacement. Refused writes (verified mode), missing features and the single-user,
  one-iPhone verification must be visible.
- Docs that contradict each other or the code. Where a doc and the code disagree, the code (and
  its tests) is checked and the doc follows.
- Decompilation details (addresses, decompiled symbol names, tooling, binary hashes, app
  internals) left anywhere in the current tree.

## Decided by the user

- Audience and README role as above.
- The as-is docs live directly in `docs/`, rewritten from the raw material.
- The reference material is kept and organised a bit, not rewritten.
- `docs/superpowers/` stays where it is.
- The TUI is documented in **one** page: `docs/tui.md` absorbs `docs/design/tui-layout.md`.
- Licence: MIT.
- Reverse-engineering notes: document the behaviour only, without decompilation details. Remove
  them in a new commit; no history rewrite.
- Work directly on `master`, no worktree.
- Platforms: the README says Linux and macOS, with a plain note that macOS is not tested yet.
- The scan's small leftovers stay: aggregate counts of real task shapes in the phase-0 logs, the
  acceptance labels P1/P2/L1, and `Europe/Warsaw` in tests.
- A VHS recording like unifictl's is wanted; it may be deferred if it is too much work, in which
  case a screenshot from fake data is enough for v1.

## The design

### File layout after the change

```
README.md                 front page (rewritten)
LICENSE                   MIT, copyright Bartłomiej Wilczyński, 2026
AGENTS.md, CLAUDE.md      stay at the root; links updated to the new paths
docs/
  README.md               index: the as-is pages, then the reference material
  install.md              building and installing, first run against a vdir
  tui.md                  the screen and every key (absorbs design/tui-layout.md)
  tasks.md                how tasks behave (see below)
  sync.md                 CalDAV + vdirsyncer setup, `tdo sync`, conflicts, limits
  compatibility.md        living alongside 2Do; verified mode; the matrix; migrating
  data-model.md           how a task is stored on disk
  architecture.md         crates, store, journal and recovery, locking (absorbs journal.md)
  config.md               configuration reference (revised in place)
  demo/                   VHS tapes, synthetic demo data generator, render script, GIFs
  reference/
    README.md             what the reference is and how it relates to docs/
    2do/                  2Do reference 01–06 (decompilation details stripped; 07 deleted)
    phase0/               moved from docs/phase0/ (gate checklist and session logs)
  superpowers/            unchanged location (spec, plans)
```

`docs/design/` and `docs/journal.md` disappear (absorbed). `compat/matrix.toml` stays where it is
(code and tests read it).

### README

In this order:

1. One-paragraph pitch: keyboard-driven terminal task manager for Linux and macOS (macOS not
   tested yet), VTODO over
   CalDAV, so the 2Do iPhone app keeps working on the same data. Free, MIT.
2. The demo GIF (or screenshot).
3. **Status**: a short "Works today" list and a "Not yet" list (adding tasks in the TUI with
   natural language, the notifier, manual order, list management; where they are headed). One
   sentence on verified mode: writes the compatibility matrix has not proven safe against 2Do are
   refused with a reason. One sentence that it has been verified against one iPhone running 2Do.
4. **How it works with 2Do**: 2Do on the iPhone syncs over CalDAV (not Dropbox) to a CalDAV
   server; vdirsyncer mirrors that to a local folder; `tdo` reads and writes the folder. A small
   ASCII diagram is welcome.
5. **Install and quick start**: short, links to `docs/install.md` and `docs/sync.md`.
6. **Documentation**: links to every page in `docs/`, one line each.
7. Licence line; a note that Twydo is not affiliated with 2Do or Beehive Innovations.

### The as-is pages

Each page describes current behaviour, checked against the code and tests, written for a user
(or contributor for `architecture.md`, `data-model.md`). Sources to draw on are listed; the
builder reads them and the code, and writes fresh text rather than pasting.

| Page | Covers | Draw from |
|---|---|---|
| `install.md` | Toolchain (`mise.toml`, `rust-toolchain.toml`), `cargo install --path crates/tdo` or `--git`, where config and state live, pointing `tdo` at a vdir, first run | `docs/config.md`, current README, `crates/tdo` CLI help |
| `tui.md` | Layout (sidebar, list, details), groups, focus, themes, every key, the edit form, notes in `$EDITOR`, what a refused write looks like | `docs/tui.md`, `docs/design/tui-layout.md`, the TUI code |
| `tasks.md` | Plain tasks, checklists vs projects and what completing/reopening a parent or child does; repeats (daily … yearly, from completion, last day of month, due time, start date, missed occurrences, how a completed repeat moves); date groups and sorting; alerts as far as Twydo handles them | 2Do reference 01 (§6 repeat table, §12 groups/sort), `docs/phase0/plan6-baseline.md`, `docs/phase0/gap-session.md`, core code and tests |
| `sync.md` | Revise in place: setup, `tdo sync`, the vdir lock, conflicts, limits | existing `docs/sync.md`, `docs/phase0/plan3-sync.md` |
| `compatibility.md` | Coexisting with 2Do: switching 2Do from Dropbox to CalDAV, what is preserved untouched, verified mode and how to read a refusal, the compatibility matrix in plain words (what is verified, what is not and why) | spec §6 and §10, `compat/matrix.toml`, `docs/phase0/README.md`, 2Do reference 05/06 |
| `data-model.md` | VTODO properties Twydo reads and writes, the `X-2DOAPP-*` properties and `<2Do Meta>` JSON, how parents, children, checklists and repeats are encoded, preservation of unknown properties | spec §6, 2Do reference 02 and 06 (behaviour/format only), `twydo-core` model code |
| `architecture.md` | The two crates and their modules, the vdir store, the operation journal and recovery (all of `docs/journal.md`), locking, how the TUI, `tdo op` and `tdo sync` share the core | `docs/journal.md`, the code |
| `config.md` | Revise in place so it matches the code | existing `docs/config.md`, config code |

### Reference material

- `docs/reference/README.md`: one short page saying what is here (2Do reference, phase-0 gate and
  session logs, plus a pointer to `docs/superpowers/` for the spec and plans) and that the as-is
  docs in `docs/` win where they differ.
- `docs/phase0/` moves to `docs/reference/phase0/` (`git mv`, content unchanged).
- `docs/reference/2do/` stays, content unchanged **except** removing decompilation details:
  - delete `07-reverse-engineering-notes.md`;
  - in 01, 02, 04, 06 and its README, remove decompiled symbol names, code addresses
    (`0x…`), tool names (Ghidra, REA, otool), binary paths/hashes and framework internals (e.g.
    `TodoDAOFramework`, `SyncEngine`, `TDMMainMenu.nib`). Keep the observed behaviour and
    formats. A source tag like `[obs: decompiled X]` becomes `[obs]`;
  - the README's provenance paragraph describes sources as "observation of the app's behaviour
    and its sync data" without naming tooling; drop the addresses note;
  - `.gitignore`: remove the `*.ghidra-snapshot` line.
  - Finish with a grep over the whole tree (excluding `Cargo.lock`) for `ghidra|decompil|
    0x[0-9a-f]{5,}|\bREA\b|otool|TodoDAOFramework|arm64` returning nothing relevant.
- Every link in the repo is updated for the moves (README, AGENTS.md, the spec, plans, reference
  files, code comments, `compat/matrix.toml` comments). A link check (all relative Markdown links
  resolve) is part of done.

### Demo

Following unifictl's setup (`~/Projects/homelab/unifictl/docs/demo/`: `common.tape`, one tape
per GIF, `render.sh`, a `mise run demo` task; VHS, ttyd and ffmpeg are installed):

- A demo vdir is **generated at render time** into a temporary directory from synthetic tasks,
  with dates relative to the render date, so Overdue/Today/Upcoming groups are populated. The
  content is made up and plausible (a few lists, a checklist, a project, a monthly repeat), in the
  spirit of `crates/twydo-core/tests/data/sample-vdir/`.
- `tdo` runs with a config pointing at that vdir and an isolated state dir; it must never touch
  the user's real config, vdir or state.
- One tape is enough for v1: open the TUI, move between lists, open a task's details, complete a
  task, edit one. Fixed terminal size and theme; `TZ` fixed.
- If VHS proves impractical, fall back to a PNG screenshot made the same way (same generated
  data) and leave a follow-up note.

### Licence

`LICENSE` with the MIT text; `license = "MIT"` in the workspace `Cargo.toml` (inherited by both
crates).

### Going public

The final step, after the user has reviewed the docs: `gh repo edit bwilczynski/twydo
--visibility public --accept-visibility-change-consequences`. **Ask the user before running
it.** Before asking, re-run the personal-data scan over the current tree and history (already run
once on 2026-11-02 across all 154 commits: clean apart from placeholders, test pangrams, the
`Europe/Warsaw` time zone in tests, aggregate shape counts of the real data and acceptance case
labels, which the user may want to know about).

### Calls made while writing this up (not the user's decisions)

- `AGENTS.md` stays an agent/contributor handoff. Its links and "done so far" status are updated;
  it is not rewritten for the public. `../rea-private/` is a local directory name, not tooling, and
  stays; the decompilation grep is case-sensitive (`\bREA\b`) so it does not match.
- The demo runs in the default verified mode. Its data uses only shapes the matrix verifies for
  the writes the tape performs (plain tasks, a monthly repeat), so nothing is refused on camera.
- `tools/` scripts are documented briefly: `decode_2do_dropbox.py` in the migration section of
  `compatibility.md`; `capture_fixture.py`, `seed_fixture.py`, `reconcile.py` in
  `docs/reference/README.md` as phase-0/testing tools.
- The personal-data scan is pattern-based (URLs, emails, IPs, home paths, UUIDs, Polish
  characters, every `SUMMARY`/`DESCRIPTION`/`LOCATION`/list name in history, device and server
  names). The private acceptance file is not on this machine, so the user does the final check of
  real titles by eye. It is re-run before going public.
- This design document is committed with the work.

## Left to the builder

- Wording, page lengths, and the internal headings of each page.
- The exact demo script and data, within the constraints above.
- Whether a fact belongs in `tasks.md` or `data-model.md` when it straddles both (behaviour in
  `tasks.md`, encoding in `data-model.md`, cross-linked).
- Commit granularity on `master` (conventional commit messages, as in the existing history).
