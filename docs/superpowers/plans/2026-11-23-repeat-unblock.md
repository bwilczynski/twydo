# Repeat Unblock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** "After done" repeats at any interval on tasks, projects and checklists; project repeat
changes; completing after-done projects; weekly, interval and ending recurring checklists;
after-done tasks with a start date (#172, #174, part of #29).

**Architecture:** Part 1 widens `gate::captured_from_completion_edit` for standalone tasks on
existing evidence. Part 2 adds the family and start-date guards behind new or existing matrix
contracts that stay `unverified` in `compat/matrix.toml`; a private candidate matrix lets the
branch daemon seed and complete the round's twins. After one phone round, each contract flips to
`pass` only on its own observed match, backed by promoted fixtures and twin tests.

**Tech Stack:** Rust workspace (`twydo-core`, `tdo`), Python evidence tools, vdirsyncer through
the daemon, 2Do on the iPhone.

**Spec:** `docs/superpowers/specs/2026-11-23-repeat-unblock-design.md`

## Global Constraints

- Device scope is exactly the spec's round (P1, P2, C1–C3, T1): the production account's
  synthetic "Twydo test" list only; no other list, account or global write-mode change; the user
  approves the Task 5 checklist before anything is uploaded.
- The phone does no typing and creates nothing; Twydo seeds every task.
- Private staging, backups, aliases, notes and the candidate matrix live in
  `~/twydo-repeat-round/`, never in git. Every staged state passes
  `tools/capture_fixture.py scan` and the user confirms it is synthetic before promotion.
- No new contract is `pass` in `compat/matrix.toml` before its own round result matches.
- Encodings are unchanged: Twydo writes every rule exactly as today.
- Projects that end, yearly checklists, timed starts on after-done repeats and starts on
  after-done families stay refused.
- Commits pass commitlint (`type(scope): subject`, lowercase subject, ≤100 chars); no agent
  attribution. Cargo target dir under `~/.cache`, never `/tmp`. Tests isolate `XDG_CONFIG_HOME`
  and `TWYDO_SOCKET`.

## Review Focus

1. A mixed card save (title plus a refused repeat) on a project must write nothing. Test:
   `gate::project_repeat_mixed_save_with_refused_rule_refuses_whole_save` (Task 3).
2. An after-done project with a cancelled item must refuse completion with a reason, not
   half-complete. Test: `gate::after_done_project_with_cancelled_item_refuses_completion` (Task 3).
3. Weekly due → weekly after done stays refused for tasks and projects (not captured). Test rows
   in `verified_from_completion_repeat_changes_require_captured_transitions` (Task 1) and
   `project_repeat_transitions_follow_the_captured_table` (Task 3).
4. A checklist due on the 29th–31st with a new variant rule stays refused. Test row in
   `checklist_variants_admit_only_the_composed_shapes` (Task 4).
5. Undo of a project repeat edit restores the exact previous bytes. Test:
   `store_family::undo_project_repeat_edit_restores_bytes` (Task 3).

---

### Task 1: After-done edits at any interval on standalone tasks (#174, no phone)

**Files:**
- Modify: `crates/twydo-core/src/gate.rs` (`captured_from_completion_edit`, ~line 1399)
- Test: `crates/twydo-core/tests/gate.rs` (the two `verified_from_completion_*` tests, ~line 1500)
- Modify: `docs/compatibility.md` (table row "Repeat from completion", footnote ² row)

**Interfaces:**
- Produces: `fn fc_transition_ok(before: Option<(Freq, bool)>, after: Option<(Freq, bool)>) -> bool`
  in `gate.rs`, the spec's transition table with interval ignored. Task 3 reuses it for projects.

- [ ] **Step 1: Write the failing tests.** In `verified_from_completion_edit_rows_do_not_enable_uncaptured_shapes`
  add rows: `("3m after done", true)`, `("every 3 months after done", true)`, `("2w after done", true)`,
  `("3d after done", false)`, `("2y after done", false)`, `("3m after done until 2027-06-01", false)`;
  change `("FREQ=MONTHLY", Change::RepeatInterval(3), false)` to `true` and add
  `("FREQ=WEEKLY", Change::RepeatInterval(3), true)` with `from_completion = true` set on the task.
  In `verified_from_completion_repeat_changes_require_captured_transitions` add:
  `(None, false, Some("3m after done"), true)`, `(Some("FREQ=MONTHLY"), false, Some("3m after done"), true)`,
  `(Some("FREQ=MONTHLY;INTERVAL=2"), true, Some("monthly after done"), true)`,
  `(Some("FREQ=WEEKLY;INTERVAL=2"), true, Some("weekly"), true)`,
  `(Some("FREQ=WEEKLY"), false, Some("2w after done"), false)`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate verified_from_completion`. Expected: the new rows FAIL.
- [ ] **Step 3:** In `captured_from_completion_edit`: `plain` ignores the interval (compare a
  clone with `interval = 1`); the `Repeat`-only branch uses `fc_transition_ok` on
  `(freq, from_completion)` pairs (the table in the spec, plus "same frequency after done, other
  interval"); drop the `interval_two` special case; `RepeatInterval(n)` is admitted for any `n`
  on a plain monthly or weekly after-done rule (`written_interval` still refuses what Twydo
  cannot complete). Update `REASON` to "from-completion edits are verified only for standalone
  monthly/weekly rules without an end, within the captured anchor changes".
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: pass (fix any older assertion that
  pinned the interval-2 limit, and its message).
- [ ] **Step 5:** Update `docs/compatibility.md` ("Monthly only²" → "Monthly/weekly²", footnote ²
  says any interval). Run `python3 tools/check_links.py`. Commit
  `feat(gate): allow after-done edits at any interval`.

### Task 2: New contracts, unverified

**Files:**
- Modify: `crates/twydo-core/src/compat/contracts.rs` (`contracts!` list), `compat/matrix.toml`
- Test: `crates/twydo-core/tests/contracts.rs`, `crates/twydo-core/tests/gate_contract_parity.rs`

**Interfaces:**
- Produces: `ContractId::CompleteRecurringProjectFromCompletion` (`"complete.recurring-project-from-completion"`,
  Complete, "completing a recurring project that repeats from completion"),
  `ContractId::CreateRecurringChecklistVariant` (`"create.recurring-checklist-variant"`, Create,
  "creating or converting a weekly, interval or ending recurring checklist"),
  `ContractId::CompleteRecurringChecklistVariant` (`"complete.recurring-checklist-variant"`, Complete,
  "completing a weekly, interval, after-done or ending recurring checklist"),
  `ContractId::SetRepeatFromCompletionStart` (`"set-repeat.from-completion-start"`, SetRepeat,
  "repeating from completion a task with a start date"). All four rows `unverified` in all three
  directions, `evidence = []`, notes naming the round. Existing `set-repeat.project` and
  `set-repeat.checklist-interval` rows are unchanged here.

- [ ] **Step 1:** Add the ids and rows. Run `cargo test -p twydo-core --test contracts --test gate_contract_parity`
  and `python3 tools/check_evidence.py`. Expected: pass (add the ids wherever the parity test
  enumerates rows, matching existing entries).
- [ ] **Step 2:** Commit `feat(compat): add unverified contracts for repeat variants`.

### Task 3: Project repeat changes and after-done project completion, behind contracts

**Files:**
- Modify: `crates/twydo-core/src/gate.rs` (`captured_from_completion_edit` family branch; the
  `PROJECT_END` block in `check_with`, ~line 418)
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/edit_complete.rs`,
  `crates/twydo-core/tests/store_family.rs`

**Interfaces:**
- Consumes: `fc_transition_ok` (Task 1), `ContractId::CompleteRecurringProjectFromCompletion` (Task 2).
- Produces: a test helper `fn candidate() -> Matrix` in `tests/gate.rs`: the embedded matrix with
  the Task 2 rows plus `set-repeat.project` and `set-repeat.checklist-interval` set to `pass`.
  Tasks 4 and 5 reuse it (Task 5 writes the same rows to the private candidate matrix file).

- [ ] **Step 1: Write the failing tests** (build projects with the synthetic helpers already in
  `tests/gate.rs`; a top-level flat project, two plain items, date-only due day 10):
  - `project_repeat_transitions_follow_the_captured_table`: with `candidate()` in Verified mode,
    plain monthly → "3m after done" ok; "3m after done" → "monthly" ok; → none ok; none →
    "monthly after done" ok; weekly → "2w after done" refused; "monthly after done until 2027-06-01"
    refused. With `Matrix::embedded()` every row is refused with "changing a project's repeat rule".
  - `project_repeat_mixed_save_with_refused_rule_refuses_whole_save`: `Op::Save` with a title and
    "2w after done" on a weekly project is refused.
  - `after_done_project_completion_needs_its_contract`: completing a "3m after done" project is
    refused with `embedded()` (PROJECT_END text) and admitted with `candidate()`; an ending
    project stays refused with both.
  - `after_done_project_with_cancelled_item_refuses_completion` (`candidate()`).
  - `edit_complete::after_done_project_completion_advances_from_today`: completing the family on
    2026-11-23 moves the live due to 2027-02-23, writes a completed family copy and reopens items
    (use the existing family completion helper the recurring-project tests use).
  - `store_family::undo_project_repeat_edit_restores_bytes`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate --test edit_complete --test store_family project`.
  Expected: new tests FAIL.
- [ ] **Step 3:** In `captured_from_completion_edit`, admit a top-level declared project with only
  plain-task children when the transition passes `fc_transition_ok` (same due/alarm guards as a
  task; children are not checked for alarms). In the `PROJECT_END` block, an after-done project
  with no end passes when `require(m, mode, CompleteRecurringProjectFromCompletion)` passes and it
  has no cancelled child; otherwise keep today's message.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: pass. Commit
  `feat(gate): admit project repeat changes behind the project contract`.

### Task 4: Recurring checklist variants and after-done starts, behind contracts

**Files:**
- Modify: `crates/twydo-core/src/gate.rs` (`captured_checklist_write`, `checklist_repeat_shape`,
  the recurring-checklist completion guards in `check_with` ~line 420–455, the `date_only`
  requirement in `captured_from_completion_edit`)
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/edit_complete.rs`

**Interfaces:**
- Consumes: `candidate()` (Task 3); Task 2 ids.
- Produces: `fn checklist_variant(r: &RRule, from_completion: bool, due: Option<NaiveDate>) -> Option<bool>`:
  `Some(false)` for the already-verified plain monthly interval-1 shape, `Some(true)` for a
  variant ({weekly, monthly} × any interval × {due, after done}, or monthly interval 1 with a
  date-only `UNTIL`; due day 1–28), `None` otherwise.

- [ ] **Step 1: Write the failing tests:**
  - `checklist_variants_admit_only_the_composed_shapes` (`candidate()`, Verified): create
    checklists "2w", "monthly after done", "3m after done", "monthly until <due>", "2m" → ok;
    "yearly", "3d", "monthly" due on the 30th with "2w", "2w until …" → refused. Converting a
    "3m after done" task to a checklist → ok. Completing each admitted shape → ok; with a cancelled
    item → refused. With `embedded()` every variant is refused and plain monthly still passes.
  - Interval edits: changing a checklist "2w" → "3w" needs `set-repeat.checklist-interval`
    (refused with `embedded()`, ok with `candidate()`).
  - `after_done_with_start_needs_its_contract`: setting "3m after done" on a standalone task with
    a date-only start 3 days before its due is refused with `embedded()`, ok with `candidate()`;
    a timed start stays refused with both.
  - `edit_complete::after_done_checklist_completion_advances_from_today`: a "3m after done"
    checklist completed on 2026-11-23 → live due 2027-02-23, items reset, history copy as for
    monthly checklists.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate --test edit_complete checklist`. Expected: FAIL.
- [ ] **Step 3:** Use `checklist_variant` in the three checklist guards; a `Some(true)` shape
  additionally requires `CreateRecurringChecklistVariant` (create, conversion, repeat change) or
  `CompleteRecurringChecklistVariant` (completion); any interval ≠ 1 on a checklist also requires
  `SetRepeatChecklistInterval`. In `captured_from_completion_edit`, a date-only start no longer
  fails `date_only` when `SetRepeatFromCompletionStart` passes (pass the matrix and mode in).
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: pass. Commit
  `feat(gate): admit checklist repeat variants behind contracts`.

### Task 5: Round preparation (stops for approval)

**Files:** none in the repo; private `~/twydo-repeat-round/` (`candidate-matrix.toml`,
`seed.sh`, `aliases.txt`, `notes.md`, `checklist.md`).

**Interfaces:**
- Consumes: Tasks 1–4 built with `cargo build --release -p tdo`.
- Produces: the candidate matrix (`compat/matrix.toml` with the six rows from `candidate()` at
  `pass`); `seed.sh` that creates, through `tdo op add`/`tdo op edit`/`tdo op complete` against the
  running branch daemon, every seed in the spec table with synthetic titles `P1a repeat`,
  `P1b repeat`, `P2 repeat`, `C1a repeat` … `T1b repeat` and items `item 1`, `item 2` (item 1
  completed). Dues: P1/P2/C2/T1 2026-12-10, C1 2026-11-30, C3 2026-12-10 with until 2026-12-10;
  T1 start 2026-12-07. P1 is created as plain monthly then edited to `3m after done`; C2 is
  created as a task with `3m after done` then converted with `--kind checklist`; T1 is created
  with its start then edited to `3m after done`.

- [ ] **Step 1:** Write the candidate matrix and `seed.sh`; run `seed.sh` against an isolated test
  daemon (`XDG_CONFIG_HOME`, `TWYDO_SOCKET` in `~/.cache/twydo-repeat-dry/`) to prove every op is
  admitted; stop that daemon with `tdo daemon stop`.
- [ ] **Step 2:** Write `checklist.md`: backup, stop `twydo.service`, branch daemon with
  `TWYDO_NOTIFIER=off tdo daemon --matrix ~/twydo-repeat-round/candidate-matrix.toml`, seed, sync,
  capture `twydo-seeded`; then the spec's phone steps 1–5 with the capture after each
  (`phone-p2-and-view`, `phone-completed`, `twydo-completed`, `phone-checked`); restore the
  mise `tdo` and `twydo.service`.
- [ ] **Step 3: Checkpoint.** Show the user `checklist.md`, the list of phone actions, and the
  rollback. Wait for explicit approval before Task 6.

### Task 6: The round (human-in-the-loop)

**Files:** private staging only.

- [ ] **Step 1:** Back up `~/.local/share/twydo/vdir` and the daemon state to
  `~/twydo-repeat-round/backup-<UTC stamp>/`; stop `twydo.service`; start the branch daemon as
  in the checklist; run `seed.sh`; `tdo sync`; record UIDs in `aliases.txt`; capture
  `twydo-seeded` with `python3 tools/capture_fixture.py capture … --collection <Twydo test id>`
  and every seed `--resource` (as in the start-time round).
- [ ] **Step 2:** Ask the user to sync, open P1a/P1b and report their repeat (Q2), then set P2 to
  every 3 months after completion. `tdo sync`; capture `phone-p2-and-view`.
- [ ] **Step 3:** Ask the user to complete P1a, C1a, C2a, C3a, T1a (confirming "complete all
  items" if asked). `tdo sync`; capture `phone-completed`. Note the date.
- [ ] **Step 4:** The same day, `tdo op complete --all` P1b, C1b, C2b, C3b, T1b; sync; capture
  `twydo-completed`. Ask the user to open each b and say whether it matches its a (Q5); capture
  `phone-checked`. Record answers in `notes.md`.
- [ ] **Step 5:** Restore the mise `tdo`, restart `twydo.service`; run `capture_fixture.py scan`
  and show it to the user for synthetic confirmation.

### Task 7: Evidence, verdicts and twin tests

**Files:**
- Create: `fixtures/set-repeat/project-roundtrip/`, `fixtures/complete/project-from-completion/`,
  `fixtures/complete/checklist-variants/` (states from Task 6 and `case.toml`, as in
  `fixtures/set-start/phone-encodings/case.toml`)
- Modify: `compat/matrix.toml`, `crates/twydo-core/tests/edit_complete.rs`,
  `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/src/edit/complete.rs` (only on a mismatch
  Twydo can mirror), spec `## Round results`

- [ ] **Step 1: Twin tests.** For each of P1, C1, C2, C3, T1:
  `twin_<alias>_matches_the_phone` takes the b seed from `twydo-seeded`, completes it with the
  phone's completion date, and compares lines and meta with the phone's a from `phone-completed`
  (the `check_twin_step` pattern, identity fields masked). For P2:
  `project_repeat_set_on_the_phone_reads_as_3m_after_done` (task model of P2 in
  `phone-p2-and-view`: rule `FREQ=MONTHLY;INTERVAL=3`, `from_completion`, items and links unchanged).
- [ ] **Step 2:** Run them. Expected: pass for matching shapes. A mismatch Twydo can mirror
  (for example the start moving differently for T1) is fixed in `complete.rs` with its twin test
  as the proof; anything else leaves that shape refused and goes into `## Round results` as a
  known failure.
- [ ] **Step 3:** Promote the cases (scan clean, user confirmed). Flip to `pass` only the rows
  whose shapes matched: `set-repeat.project` (P1 view + P2), `complete.recurring-project-from-completion`
  (P1), `create.recurring-checklist-variant` and `complete.recurring-checklist-variant` (C1–C3),
  `set-repeat.checklist-interval` (C1), `set-repeat.from-completion-start` (T1); each with its
  case in `evidence`. If one checklist shape failed, narrow `checklist_variant` to exclude it with
  a test row before flipping. Run `python3 tools/check_evidence.py` and `cargo test --workspace`.
  Expected: pass. Commit `test(evidence): verify project and checklist repeat variants`.

### Task 8: Docs and finish

**Files:**
- Modify: `docs/compatibility.md` (table rows Recurring checklist / Recurring project / Repeat
  from completion; the newly enabled operations table), `docs/tasks.md` (~line 244, the refused
  recurring-project sentence), `docs/reference/evidence.md` (remaining restrictions),
  `AGENTS.md` ("Current work": this round complete), spec status.

- [ ] **Step 1:** Update the pages. Run `python3 tools/check_links.py`. Expected: pass.
- [ ] **Step 2:** Run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, and
  commitlint from `origin/main`. Expected: pass.
- [ ] **Step 3:** Commit `docs: describe repeat variants and project repeat changes`. The PR body
  closes #172 and #174 and references #29 with what remains (yearly, ending projects).
