# Public Repository and Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `bwilczynski/twydo` ready to be public: a front-page README with a demo, as-is topic docs in `docs/`, organised reference material without decompilation details, and an MIT licence.

**Architecture:** Mostly documentation. Two small Python tools carry the testable parts: a Markdown link checker (gates every doc task) and a demo-vdir generator (shifts the synthetic sample vdir to "today" for the VHS recording). Docs are rewritten from the raw material and checked against the code.

**Tech Stack:** Markdown, Python 3 stdlib + `unittest` (tools), VHS + ttyd + ffmpeg (demo), Rust workspace (`cargo test`).

**Spec:** `docs/superpowers/specs/2026-11-02-public-repo-docs-design.md`

## Global Constraints

- Work directly on `master`. Conventional commit messages (`docs: …`, `feat(tools): …`, `chore: …`). **No agent attribution** in commits: no `Co-Authored-By`, no session links (user's global instructions override the harness).
- No personal data in the repository or the demo: no real task titles, list names, dates of real obligations, names, CalDAV URLs or account details. Demo data is synthetic only.
- No history rewrite: no `git filter-repo`, no force-push.
- `docs/superpowers/` stays where it is; spec and plans are not rewritten (only links fixed).
- Reference material is not rewritten, except removing decompilation details.
- README: Linux and macOS, with a plain note that macOS is not tested yet. Not affiliated with 2Do or Beehive Innovations.
- Leftovers stay: aggregate real-data counts in the phase-0 logs, labels P1/P2/L1, `Europe/Warsaw` in tests.
- Where a doc and the code disagree, the code (and its tests) wins and the doc follows.
- Every task ends with: `python3 tools/check_links.py` exit 0, and `cargo test --workspace` passing when the task touched anything under `crates/` or `compat/`.
- Going public (`gh repo edit … --visibility public`) only after asking the user.

## Review Focus

1. **A moved file's own relative links** (`docs/phase0/*` → `docs/reference/phase0/*` gains one `../` level): the link checker must cover links inside moved files, not just links to them (Task 2 step 4).
2. **Old paths in code spans and code comments** (`` `docs/phase0/…` ``, `docs/journal.md` in `//!` comments, `compat/matrix.toml` notes) are not Markdown links, so the link checker misses them: a grep gate in Task 2 and Task 9.
3. **Demo writes refused in verified mode**: a shifted repeat landing on day 29–31 falls into an unverified rule class. The generator clamps (Task 4 test `test_monthly_day_clamped_to_28`), and Task 5 runs the tape's writes through `tdo op` in verified mode first.
4. **The demo touching real config or state**: render must set `XDG_CONFIG_HOME` and `XDG_STATE_HOME` to a temp dir (Task 5 step 3 checks the real state dir is untouched).
5. **Docs overselling or contradicting the code**: every "works today" claim in README and `docs/` is traced to a command, key or test (Task 9 review checklist).

---

### Task 1: Licence and link checker

**Files:**
- Create: `LICENSE`, `tools/check_links.py`, `tools/tests/test_check_links.py`
- Modify: `Cargo.toml` (`[workspace.package]`), `crates/twydo-core/Cargo.toml`, `crates/tdo/Cargo.toml`

**Interfaces:**
- Produces: `python3 tools/check_links.py [ROOT]` → prints `path:line: broken link <target>` per broken link, exit 1 if any, else exit 0. Module function `broken_links(root: Path) -> list[tuple[Path, int, str]]`.

- [ ] **Step 1: Write failing tests** in `tools/tests/test_check_links.py` (same `importlib` loading pattern as `test_seed_fixture.py`), each building a temp tree:
  - `test_relative_link_ok`: `a.md` links `[x](sub/b.md)`, `sub/b.md` exists → `broken_links == []`.
  - `test_missing_target_reported`: `[x](nope.md)` on line 3 → `[(Path("a.md"), 3, "nope.md")]` (paths relative to root).
  - `test_anchor_and_external_ignored`: `[x](https://e.com)`, `[x](#sec)`, `[x](mailto:a@b.c)` → `[]`; `[x](b.md#sec)` with `b.md` present → `[]`.
  - `test_link_to_directory_ok`: `[x](sub/)` with `sub/` present → `[]`.
  - `test_skips_target_and_git`: a broken link inside `target/x.md` and `.git/x.md` is not reported.
  - `test_link_inside_code_fence_ignored`: a broken link between ```` ``` ```` fences is not reported.
- [ ] **Step 2: Run** `python3 -m unittest tools.tests.test_check_links -v` → FAIL (module missing).
- [ ] **Step 3: Implement `tools/check_links.py`**: walk `*.md` under root (skip `target/`, `.git/`, `node_modules/`), find inline links `](target)` outside fenced blocks, skip schemes and pure anchors, strip `#…`, URL-decode, resolve against the file's directory.
- [ ] **Step 4: Run** the unit tests → PASS. Run `python3 tools/check_links.py` on the repo and note the current broken links (expected few or none; fix none yet, later tasks own their files).
- [ ] **Step 5: Licence.** `LICENSE`: standard MIT text, `Copyright (c) 2026 Bartłomiej Wilczyński`. Add `license = "MIT"` to `[workspace.package]` and `license.workspace = true` to both crates. `cargo test --workspace` → PASS.
- [ ] **Step 6: Commit** `chore: add MIT licence and a Markdown link checker`. First commit the design and this plan separately: `docs: design and plan for the public repository`.

### Task 2: Move phase-0 into the reference and fix every path

**Files:**
- Move: `docs/phase0/` → `docs/reference/phase0/` (`git mv`, content unchanged apart from links)
- Create: `docs/reference/README.md`
- Modify: every file referencing `docs/phase0` (found by grep; today: `AGENTS.md`, `README.md`, `compat/matrix.toml` notes, code comments in `crates/twydo-core/src/{store,edit}/family.rs`, `crates/twydo-core/tests/store_family.rs`, `docs/sync.md`, `docs/tui.md`, `docs/config.md`, spec and plans, files inside `docs/reference/`). Note `fixtures/phase0/` is a different path and **does not move**.

- [ ] **Step 1:** `git mv docs/phase0 docs/reference/phase0`.
- [ ] **Step 2:** Replace `docs/phase0/` with `docs/reference/phase0/` everywhere (not `fixtures/phase0/`). Fix relative links from and to the moved files.
- [ ] **Step 3:** Write `docs/reference/README.md`: what is here (`2do/` behaviour and format reference; `phase0/` gate checklist and phone-session logs; spec and plans in `../superpowers/`), that the as-is docs in `docs/` win where they differ, and the `tools/` scripts: `capture_fixture.py`, `seed_fixture.py`, `reconcile.py` (phase-0 and testing), `decode_2do_dropbox.py` (Dropbox inventory, also linked from `compatibility.md` later).
- [ ] **Step 4: Verify.** `python3 tools/check_links.py` → exit 0. `git grep -n 'docs/phase0'` → no output. `cargo test --workspace` → PASS (matrix notes are compiled in).
- [ ] **Step 5: Commit** `docs: move phase-0 logs into docs/reference`.

### Task 3: Remove decompilation details

**Files:**
- Delete: `docs/reference/2do/07-reverse-engineering-notes.md`
- Modify: `docs/reference/2do/{README,01-features,02-data-model,04-keyboard-shortcuts,06-sync}.md`, `.gitignore`

- [ ] **Step 1:** `git rm` file 07; remove its row from `docs/reference/2do/README.md` and any links to it.
- [ ] **Step 2:** In the listed files, remove decompiled symbol names, code addresses (`0x…`), tool names (Ghidra, REA, otool), binary paths and hashes, and framework internals (`TodoDAOFramework`, `SyncEngine`, `SyncEngineCoordinator`, `TDMMainMenu.nib`, `Assets.car`, `Localizable.strings`). Source tags become plain markers (`[obs: decompiled X]` → `[obs]`, `[obs: folder enumeration at 0x…]` → `[obs]`). Keep every behaviour and format statement. Leave UI words like "list selector" alone.
- [ ] **Step 3:** Rewrite the README's provenance paragraph: sources are observation of the app's behaviour, its UI and its sync data; no tooling named. Delete the addresses note.
- [ ] **Step 4:** Remove `*.ghidra-snapshot` from `.gitignore`.
- [ ] **Step 5: Verify.** `git grep -n -E 'ghidra|Ghidra|decompil|0x[0-9a-f]{5,}|\bREA\b|otool|TodoDAOFramework|arm64|\.nib\b|Assets\.car|Localizable' -- . ':!Cargo.lock'` → no output (`rea-private` in `AGENTS.md` does not match `\bREA\b` case-sensitively). `python3 tools/check_links.py` → exit 0.
- [ ] **Step 6: Commit** `docs(reference): keep 2Do's behaviour, drop decompilation details`.

### Task 4: Demo vdir generator

**Files:**
- Create: `tools/demo_vdir.py`, `tools/tests/test_demo_vdir.py`

**Interfaces:**
- Consumes: `crates/twydo-core/tests/data/sample-vdir/` (synthetic, anchored at Wed 2026-11-04, `TZID=America/New_York`).
- Produces: `python3 tools/demo_vdir.py OUT_DIR [--today YYYY-MM-DD]` copies the sample vdir into `OUT_DIR` with every date shifted by `today - 2026-11-04` days. Function `shift_ics(text: str, days: int) -> str`.

- [ ] **Step 1: Write failing tests** in `tools/tests/test_demo_vdir.py`:
  - `test_date_value_shifted`: `DUE;VALUE=DATE:20261105` shifted by 10 → `DUE;VALUE=DATE:20261115`.
  - `test_datetime_shifted_keeps_tzid_and_time`: `DUE;TZID=America/New_York:20261104T150000` by 30 → `…:20261204T150000`.
  - `test_utc_stamps_shifted`: `COMPLETED:20261102T170000Z` by 1 → `20261103T170000Z` (also `DTSTART`, `CREATED`, `DTSTAMP`, `LAST-MODIFIED`, `TRIGGER;VALUE=DATE-TIME` if present).
  - `test_weekly_byday_follows_due`: `DUE;VALUE=DATE:20261104` + `RRULE:FREQ=WEEKLY;BYDAY=WE` by 1 → `BYDAY=TH`.
  - `test_monthly_bymonthday_follows_due`: `20261107` + `BYMONTHDAY=7` by 3 → `20261110` and `BYMONTHDAY=10`.
  - `test_monthly_day_clamped_to_28`: `20261107` + `BYMONTHDAY=7` by 23 (→ 30th) → due `20261128`, `BYMONTHDAY=28`.
  - `test_relative_trigger_untouched`: `TRIGGER:-PT15M` unchanged.
  - `test_generates_whole_vdir`: run `main([tmp, "--today", "2026-11-04"])` → output tree equals the sample vdir byte for byte (offset 0); with `--today 2026-12-01` every collection, `displayname` and `color` file is present.
- [ ] **Step 2: Run** `python3 -m unittest tools.tests.test_demo_vdir -v` → FAIL.
- [ ] **Step 3: Implement** with line-based rewriting of unfolded property lines (the sample has no folded date lines; assert that rather than handle folding).
- [ ] **Step 4: Run** tests → PASS. Then `python3 tools/demo_vdir.py "$S/demo" && ./target/release/tdo --vdir "$S/demo" check` (with `S` a scratch dir) → exit 0, and `tdo --vdir "$S/demo" export-json` shows tasks due today and overdue.
- [ ] **Step 5: Commit** `feat(tools): generate a demo vdir from the synthetic sample`.

### Task 5: VHS demo

**Files:**
- Create: `docs/demo/common.tape`, `docs/demo/tui.tape`, `docs/demo/render.sh`, `docs/demo/tui.gif`
- Modify: `mise.toml` (add `vhs = "0.12.1"`, `ttyd = { version = "1.7.7", os = ["linux", "windows"] }`, `[tasks] demo = "docs/demo/render.sh"`)

**Interfaces:**
- Consumes: `tools/demo_vdir.py` (Task 4); keys from `docs/tui.md`.
- Reference setup: `~/Projects/homelab/unifictl/docs/demo/` (`common.tape` theme, font, size; `render.sh`).

- [ ] **Step 1:** `render.sh` (bash, `set -euo pipefail`, cd to repo root): check `ffmpeg`; `cargo build --release`; make a temp dir `T` (removed on exit); `python3 tools/demo_vdir.py "$T/vdir"`; write `$T/config/twydo/config.toml` with `vdir = "$T/vdir"`, `timezone = "America/New_York"`, default `[writes]` (verified), no `[sync]`; export `XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state TZ=America/New_York PATH=$PWD/target/release:$PATH`; run `vhs docs/demo/tui.tape`.
- [ ] **Step 2:** Before writing the tape, prove its writes pass verified mode with the same env: `tdo op complete <id of "Pay electricity bill">` and `tdo op edit <id of "Call the dentist"> --title "Call the dentist (moved)"` → both exit 0. If either is refused, pick another one-time task the matrix verifies.
- [ ] **Step 3:** `common.tape` (copy unifictl's look: Catppuccin Mocha, JetBrainsMono Nerd Font, padding, window bar) and `tui.tape`: start `tdo`, move between lists, open a task's details, complete "Pay electricity bill", edit one title, quit. `Output docs/demo/tui.gif`. Run `mise run demo`; check the GIF by eye (Overdue and Today groups populated, no errors). Confirm `~/.local/state/twydo` and `~/.config/twydo` mtimes are unchanged.
- [ ] **Step 4:** Fallback only if VHS cannot drive the TUI: a PNG via `Screenshot` in the tape, same data; note the follow-up in the commit body.
- [ ] **Step 5: Commit** `docs(demo): record the TUI from synthetic data with VHS`.

### Task 6: `architecture.md` and `data-model.md`

**Files:**
- Create: `docs/architecture.md`, `docs/data-model.md`
- Delete: `docs/journal.md` (absorbed)
- Modify: references to `docs/journal.md` (`crates/twydo-core/src/store/{mod,journal}.rs` doc comments, `docs/config.md`, others found by grep)

- [ ] **Step 1:** `architecture.md`: the two crates and their main modules (read `crates/*/src`), the vdir store, the operation journal and recovery (all of `docs/journal.md`, revised where the code differs), locking, how `tdo tui`, `tdo op` and `tdo sync` share the core, and verified mode's place in the write path.
- [ ] **Step 2:** `data-model.md`: VTODO properties read and written; `X-2DOAPP-*` and the `<2Do Meta>` JSON and `@key:value` tokens; parent/child encoding (projects, checklists, `RELATED-TO`); repeat encoding; preservation of unknown properties. Sources: spec §6, reference 02 and 06, `twydo-core` model and meta code. No link to `tasks.md` yet; Task 7 adds the cross-links both ways.
- [ ] **Step 3: Verify.** `git grep -n 'journal\.md'` → no output. `cargo test --workspace` → PASS. Link check exit 0.
- [ ] **Step 4: Commit** `docs: architecture and data model pages`.

### Task 7: `tasks.md` and `compatibility.md`

**Files:**
- Create: `docs/tasks.md`, `docs/compatibility.md`

- [ ] **Step 1:** `tasks.md` (and add cross-links between it and `data-model.md`), for users: plain tasks; checklists vs projects, and what completing, reopening, editing or moving a parent or a child does (with `--all`, the TUI question); repeats: daily … yearly, from completion, last day of the month, with a due time, with a start date, missed occurrences, how a completed repeat's due and start move; date groups and sort order; alerts as Twydo handles them. Sources: reference 01 (§6, §12), `docs/reference/phase0/plan6-baseline.md`, `gap-session.md`, `twydo-core` repeat and family code and tests. Each rule is checked against the code.
- [ ] **Step 2:** `compatibility.md`: switching 2Do on the iPhone from Dropbox to CalDAV; what Twydo preserves untouched; verified mode and how to read a refusal (quote a real refusal message from the code); the matrix in plain words: what is verified, what is not and why (one iPhone, single user), generated from `compat/matrix.toml` by reading it, not by hand-copying every row; migrating with `tools/decode_2do_dropbox.py`. Link `data-model.md`, `sync.md`.
- [ ] **Step 3: Verify.** Link check exit 0.
- [ ] **Step 4: Commit** `docs: task behaviour and 2Do compatibility pages`.

### Task 8: `tui.md`, `sync.md`, `config.md`, `install.md`

**Files:**
- Modify: `docs/tui.md`, `docs/sync.md`, `docs/config.md`
- Delete: `docs/design/tui-layout.md` (absorbed; `docs/design/` goes away)
- Create: `docs/install.md`
- Modify: references to `docs/design/` (AGENTS.md, spec, plans, README)

- [ ] **Step 1:** `tui.md`: one page with the layout (from `tui-layout.md`, revised to the current screen), groups, focus, themes, every key (check against the key-handling code), the edit form, notes in `$EDITOR`, what a refused write looks like. Embed or link `demo/tui.gif`.
- [ ] **Step 2:** `sync.md` and `config.md`: revise in place to match the code (`tdo --help`, config structs); remove phase-0 session language; link `install.md`, `compatibility.md`.
- [ ] **Step 3:** `install.md`: toolchain from `mise.toml` / `rust-toolchain.toml`; `cargo install --git https://github.com/bwilczynski/twydo tdo` and `cargo install --path crates/tdo`; macOS not tested yet; config and state locations; first run against a vdir (`--vdir` or config); next step `sync.md`. A note that prebuilt releases and mise install are planned.
- [ ] **Step 4: Verify.** `git grep -n 'docs/design'` → only historical mentions inside `docs/superpowers/` prose that are not links, or none. Link check exit 0.
- [ ] **Step 5: Commit** `docs: one TUI page, revised sync and config, install guide`.

### Task 9: README, docs index, AGENTS.md

**Files:**
- Modify: `README.md` (rewrite), `AGENTS.md`
- Create: `docs/README.md`

- [ ] **Step 1:** `docs/README.md`: index of the as-is pages (one line each), then the reference (`reference/README.md`), then `superpowers/` (spec and plans).
- [ ] **Step 2:** `README.md` in the spec's order: pitch (Linux and macOS, macOS not tested yet; free, MIT), `docs/demo/tui.gif`, Status (Works today / Not yet: natural-language add in the TUI, the notifier, manual order, list management; verified mode in one sentence; verified against one iPhone), How it works with 2Do (ASCII: iPhone 2Do ⇄ CalDAV server ⇄ vdirsyncer ⇄ local vdir ⇄ `tdo`), Install and quick start (short, links), Documentation (links), Licence line and "not affiliated with 2Do or Beehive Innovations".
- [ ] **Step 3:** `AGENTS.md`: update paths (`docs/reference/phase0/`, `docs/tui.md`, no `docs/design`), "Done so far" to include this plan, and a pointer to `docs/README.md`. Not rewritten otherwise.
- [ ] **Step 4: Review checklist** (Review Focus 5): every "works today" item in README maps to a command in `tdo --help`, a key in `docs/tui.md` or a test; every "not yet" item is absent from the code.
- [ ] **Step 5: Verify.** Link check exit 0. `git grep -n -E 'docs/phase0|docs/design|journal\.md|07-reverse'` → no output. `cargo test --workspace` → PASS. `python3 -m unittest discover -s tools/tests -t .` → PASS.
- [ ] **Step 6: Commit** `docs: README as a front page and a docs index`.

### Task 10: Pre-publication checks and going public

- [ ] **Step 1:** Re-run the personal-data scan over the current tree and all history (URLs, emails, IPs, home paths, UUIDs, Polish characters, every `SUMMARY`/`DESCRIPTION`/`LOCATION`/list name, device and server names), the same patterns as the 2026-11-02 scan. Expected: only placeholders, test pangrams and the accepted leftovers. Also check the GIF's frames show only sample-vdir titles.
- [ ] **Step 2:** Re-run the decompilation grep (Task 3 step 5) → no output.
- [ ] **Step 3:** `git push origin master`.
- [ ] **Step 4:** Report to the user and **ask** before making the repo public. Remind them to check real task titles by eye (the private acceptance file is not on this machine).
- [ ] **Step 5:** On a yes: `gh repo edit bwilczynski/twydo --visibility public --accept-visibility-change-consequences`; confirm with `gh repo view --json visibility`.
- [ ] **Step 6:** Update the roadmap memory: step 2 done, next issue #10.
