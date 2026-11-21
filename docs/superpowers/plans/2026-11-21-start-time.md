# Start Time Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A task's start can carry a time (`tue 10am`), read from and written to 2Do exactly as 2Do encodes it, across the TUI card, `tdo op`, the phone web app and the views.

**Architecture:** A phone capture round (phase A) fixes the encoding first. Then the read path (`<2Do Meta>` → `startdate`/`starttime` tokens → `Task::start`), the write path (`Change::Start(Option<Due>)`, `NewTask::start: Option<Due>`, `edit::set_start`), new gate contracts for setting a timed start and completing a repeat with one, and the input/CLI changes. Phase B is the second phone round. Docs follow.

**Tech Stack:** Rust/Cargo workspace (`twydo-core`, `tdo`, `twydo-daemon`), Python evidence tools, vdirsyncer through the daemon, 2Do on the iPhone.

**Spec:** `docs/superpowers/specs/2026-11-21-start-time-design.md`

## Global Constraints

- Device scope is exactly the spec's phase A steps 1–9 and phase B steps 10–12. Same setup as the alerts round: the production account's synthetic "Twydo test" list only; no other list, account or global write-mode change (AGENTS.md device boundary).
- Back up the production vdir and state before each phase; private staging and notes live in `~/twydo-start-round/`, never in git.
- No personal data in fixtures; every staged state passes `tools/capture_fixture.py scan` and the user confirms it is synthetic before promotion.
- Fixtures: `fixtures/set-start/phone-encodings/` and `fixtures/set-start/twydo-roundtrip/` with complete named states and `case.toml`; `python3 tools/check_evidence.py` passes.
- New contracts `set-start.time` and `complete.start-time` are not `pass` in `compat/matrix.toml` before phase B passes on the phone. Date-only starts keep `set-start.task` exactly as today.
- Twydo mirrors 2Do's bytes for set, change, drop-time and remove (spec Q1/Q3); every other property is preserved byte for byte.
- A non-zero `StartDayDelay` stays refused. Undecodable timed-start shapes show the date only and keep their bytes.
- Protocol: `Change::Start` and `NewTask::start` widen to `Option<Due>`; this is a break, so protocol generation goes from 3 to 4 (precedent: `fac4774 feat(daemon)!: protocol generation 3`).
- Commits pass commitlint (`type(scope): subject`, lowercase subject, ≤100 chars); no agent attribution.
- Cargo target dir under `~/.cache`, never `/tmp`. Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; stop only an isolated test daemon.

## Review Focus

1. A card opened on a task with a timed start and saved unchanged must write nothing. Test: `form::unchanged_timed_start_writes_nothing` (Task 6).
2. A task whose phone-set start time Twydo cannot decode must keep its bytes through a title edit and must not have its start rewritten to midnight. Test: `edit_fields::title_edit_keeps_undecodable_start` (Task 3).
3. A timed start at 10:00 today must leave the task Scheduled at 09:59 and In Progress at 10:00 in Today and Scheduled. Test: `views::timed_start_moves_to_in_progress_at_its_time` (Task 3).
4. Undo after setting a start time must restore the exact previous bytes. Test: `gate::undo_set_start_time_restores_bytes` (Task 5).
5. Typing a bare date on a timed start must drop only the time and keep the date; typing nothing must remove the start. Test: `form::bare_date_drops_start_time` (Task 6).

---

### Task 1: Phase A on the phone (human-in-the-loop)

**Files:** none in the repo; private staging `~/twydo-start-round/`.

**Interfaces:**
- Produces: aliases `S1`–`S9` mapped to UIDs in `~/twydo-start-round/aliases.txt`; staged states `phone-a-before`, `phone-a-batch1` (steps 1–3, 7–9 created), `phone-a-batch2` (steps 4–6, 7–8 completed); notes in `~/twydo-start-round/notes.md`.

- [ ] **Step 1:** Back up `~/.local/share/twydo/vdir` and the daemon state directory to `~/twydo-start-round/backup-<UTC stamp>/`. Find the "Twydo test" collection id in the vdir (as in `fixtures/set-alert/*/case.toml`, collection alias `list1`). Run `tdo sync`, then capture `phone-a-before` with `python3 tools/capture_fixture.py capture ~/.local/share/twydo/vdir ~/twydo-start-round/staging --operation set-start --case phone-encodings --state phone-a-before --collection <id> --device iphone --origin device --claim tbd --test tbd --action "Twydo test list before the phone sets start times"`.
- [ ] **Step 2:** Show the user this checklist and wait for each batch. All titles synthetic:

| Alias | Phone action |
|---|---|
| S1 | New task `S1 start`, start 2026-11-25 10:00, no due |
| S2 | New task `S2 start`, start 2026-11-25 10:00, due 2026-11-27 (date only) |
| S3 | New task `S3 start`, start 2026-11-25 10:00, due 2026-11-27 18:00 |
| S7 | New weekly task `S7 start`, start 2026-11-23 09:00, due 2026-11-23 18:00 |
| S8 | New weekly task `S8 start`, start 2026-11-23 09:00, due 2026-11-23 (date only) |
| S9 | New task `S9 start`, start 2027-04-06 10:00 (summer time), no due |
| — | *batch 1 ends: phone syncs; `tdo sync`; capture `phone-a-batch1`* |
| S4 | Open S1, change the start time to 14:30 |
| S5 | Open S2, remove the start time, keep the date |
| S6 | Open S3, remove the start |
| — | Complete S7 and S8 on the phone |
| — | *batch 2 ends: capture `phone-a-batch2`* |

- [ ] **Step 3:** After each batch, capture with every `S*` resource listed by `--resource`. Ask the user what 2Do showed (start labels, whether a start without due offered a time, any warning) and record it in `notes.md`.
- [ ] **Step 4:** Run `python3 tools/capture_fixture.py scan ~/twydo-start-round/staging` and show the output to the user for synthetic confirmation.

### Task 2: Phase A evidence case and findings

**Files:**
- Create: `fixtures/set-start/phone-encodings/` (states from Task 1, `case.toml`)
- Modify: `docs/superpowers/specs/2026-11-21-start-time-design.md` (new section `## Capture results`)

**Interfaces:**
- Consumes: Task 1 staged states.
- Produces: the case `set-start/phone-encodings`, and in the spec `## Capture results` with: the exact `<2Do Meta>` keys/values and any `DTSTART`/other changes for S1–S3 and S9 (Q1/Q2); the bytes for S4/S5/S6 (Q3); the completed copies and advanced live tasks for S7/S8 (Q4); and a **timed-start recognizer**: the rule that turns those keys into `(NaiveDate, Option<NaiveTime>)`, including the time zone rule S9 settles. Later tasks implement exactly this section.

- [ ] **Step 1:** Re-capture the three states into the staging root with the real `--claim`, `--test` (`crates/twydo-core/tests/edit_fields.rs::set_start_time_matches_phone_bytes`) and `--action` texts; copy into `fixtures/set-start/phone-encodings/`; fill `case.toml` provenance as in `fixtures/set-alert/phone-encodings/case.toml`.
- [ ] **Step 2:** Diff the states (`diff -r`) and write `## Capture results`, applying the spec's Outcomes rules: which shapes are admitted (no due, date-only due, timed due), whether completion keeps the time (Q4), and whether monthly/yearly follow the H11 day-count rule.
- [ ] **Step 3: Checkpoint.** Show the user the Capture results section. If Q1 shows an encoding Twydo cannot write deterministically, or one that depends on the phone's zone in a way the configured `timezone` cannot reproduce, stop and agree how to proceed.
- [ ] **Step 4:** Run `python3 tools/check_evidence.py`. Expected: pass. Commit `test(evidence): capture 2do start time encodings`.

### Task 3: Read timed starts

**Files:**
- Modify: `crates/twydo-core/src/model/meta.rs:105-160` (`parse_json_metadata`: `StartDate` and any captured key → `startdate` plus `starttime` tokens, per the recognizer), `crates/twydo-core/src/model/task.rs:270-310` (`meta_start` already builds `IcalDateTime::Floating` from the tokens; adjust `start_agrees` only if the capture shows `DTSTART` carries the time)
- Test: `crates/twydo-core/tests/model_task.rs`, `crates/twydo-core/tests/views.rs`, `crates/twydo-core/tests/edit_fields.rs`

**Interfaces:**
- Consumes: Capture results recognizer (Task 2).
- Produces: `Task::start == Some(IcalDateTime::Floating(..))` for captured timed starts; `IcalDateTime::Date` for UTC-midnight starts as today.

- [ ] **Step 1: Write failing tests:**
  - `model_task::reads_phone_start_times`: S1, S2, S3 from `phone-a-batch1` → `Floating(2026-11-25T10:00)`; S9 → `Floating(2027-04-06T10:00)`; S5 from `phone-a-batch2` → `Date(2026-11-25)`; S4 → `Floating(2026-11-25T14:30)`; no warnings.
  - `model_task::undecodable_start_reads_date_only`: a synthetic `StartDate` that is neither a UTC midnight nor a captured shape → `Date` of its UTC day plus one warning.
  - `views::timed_start_moves_to_in_progress_at_its_time`: a task with start today 10:00 is in Scheduled and not In Progress at 09:59, and In Progress in Today at 10:00 (use the existing `Clock`/snapshot helpers in `tests/views.rs`).
  - `edit_fields::title_edit_keeps_undecodable_start`: a title edit on the undecodable resource leaves `X-2DOAPP-METADATA` byte-identical.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test model_task --test views --test edit_fields`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement the recognizer in `meta.rs`; update its doc comment (it says every value is a UTC midnight).
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: pass. Commit `feat(core): read start times from 2do metadata`.

### Task 4: Write timed starts

**Files:**
- Modify: `crates/twydo-core/src/edit/mod.rs` (`Change::Start(Option<Due>)` at line 58, `NewTask::start: Option<Due>` at line 80, `set_start` at line 557), `crates/twydo-core/src/edit/create.rs` (new task with a timed start), every match on `Change::Start` / `NewTask::start` the compiler reports (`gate.rs`, `form.rs`, `store/plan.rs`, `store/family.rs`, `crates/tdo/src/cli/op.rs`), `crates/twydo-daemon/src/lib.rs` and `src/schema.rs` (generation 3 → 4), `docs/protocol.md` (generation), `schema/tdo.openrpc.json` (`tdo daemon schema > schema/tdo.openrpc.json`)
- Test: `crates/twydo-core/tests/edit_fields.rs`, `crates/twydo-core/tests/edit_create.rs`

**Interfaces:**
- Consumes: `edit::Due` (existing: `Date(NaiveDate)` / `Time(NaiveDateTime)`, floating local time); the Capture results encoding; `Ctx`'s configured time zone if the encoding is an instant.
- Produces: `Change::Start(Option<Due>)`, `NewTask { start: Option<Due>, .. }`; `fn set_start(todo: &mut Component, start: Option<Due>, tz: Tz) -> Result<(), EditError>` (drop `tz` if the encoding is wall time).

- [ ] **Step 1: Write failing tests** in `tests/edit_fields.rs`, comparing as `edit_alert.rs` does (ignoring `LAST-MODIFIED`, `DTSTAMP`, metadata modification stamps):
  - `set_start_time_matches_phone_bytes`: apply `Change::Start(Some(Due::Time(2026-11-25T10:00)))` to synthetic twins of S1/S2/S3 without a start; compare to the `phone-a-batch1` resources. Same for S9 at 2027-04-06T10:00.
  - `change_start_time_matches_phone_bytes`: S1 → 14:30 equals `phone-a-batch2` S4.
  - `drop_start_time_matches_phone_bytes`: S2 → `Due::Date(2026-11-25)` equals `phone-a-batch2` S5.
  - `remove_timed_start_matches_phone_bytes`: S3 → `None` equals `phone-a-batch2` S6.
  - `edit_create::creates_task_with_start_time`: `NewTask` with `start: Some(Due::Time(..))` produces the S1 shape.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test edit_fields --test edit_create`. Expected: FAIL (type mismatch).
- [ ] **Step 3:** Widen the types and implement `set_start` per Capture results; date-only output stays byte-identical to today. Fix call sites; `tdo op --start` keeps parsing `YYYY-MM-DD` via `parse_due` (Task 6 extends it).
- [ ] **Step 4:** Bump the generation, regenerate the schema, and run `cargo test --workspace`. Expected: pass, including `no_breaking_change_against_the_released_schema` (across generations it compares only the frozen lifecycle methods). Do not edit `released.openrpc.json`; release-plz refreshes it.
- [ ] **Step 5:** Commit `feat(core)!: start times in start edits and new tasks` with a `BREAKING CHANGE: protocol generation 4; start values carry an optional time` footer.

### Task 5: Gate contracts for timed starts

**Files:**
- Modify: `crates/twydo-core/src/compat/contracts.rs` (`SetStartTime => SetStart, "set-start.time", "setting a start time"`; `CompleteStartTime => Complete, "complete.start-time", "completing a repeating task with a start time"`), `crates/twydo-core/src/gate.rs` (edit dispatch near line 1177: `Change::Start(Some(Due::Time(_)))`, or any start change on a task whose current start is timed, requires `SetStartTime` in addition to `SetStartTask`; `recurring_complete_shape` near line 1640: a captured timed `StartDate` requires `CompleteStartTime` instead of the "unusual start date" refusal; other non-midnight shapes keep that refusal), `crates/twydo-core/src/edit/complete.rs:518-540` (move the timed start per Q4), `compat/matrix.toml` (both rows `iphone_baseline = "pass"`, others unverified, evidence `["set-start/phone-encodings"]`)
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/edit_complete.rs`

**Interfaces:**
- Consumes: Tasks 3–4.
- Produces: the refusal texts `"setting a start time is not verified yet"` and `"completing a repeating task with a start time is not verified yet"` (shipping matrix until Task 7).

- [ ] **Step 1: Write failing tests:**
  - `gate::set_start_time_needs_its_contract`: shipping matrix refuses a timed start with the text above; a matrix passing `set-start.time` admits it; a date-only start is admitted as today under both.
  - `gate::undo_set_start_time_restores_bytes`: set a timed start, undo → original bytes; redo → post-set bytes (use `tests/common/store.rs`).
  - `gate::non_zero_start_day_delay_stays_refused`: unchanged verdict with a timed start.
  - `edit_complete::completes_weekly_timed_start_like_phone`: S7 and S8 from `phone-a-batch1`, completed on the phone's completion day, equal the `phone-a-batch2` copy and live task (ignoring stamps and new UIDs as the existing completion tests do).
  - `gate::complete_timed_start_needs_its_contract`, and, if Capture results says monthly/yearly follow H11, `edit_complete::monthly_timed_start_moves_by_due_days`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate --test edit_complete`. Expected: FAIL.
- [ ] **Step 3:** Implement. If Q4 was unclear, skip the completion change and keep the refusal (spec Outcomes).
- [ ] **Step 4:** Run `cargo test -p twydo-core` and `python3 tools/check_evidence.py`. Expected: pass. Commit `feat(gate): admit start times behind set-start.time`.

### Task 6: Start time input in the card, CLI and phone, and display

**Files:**
- Modify: `crates/twydo-core/src/form.rs:486-491` (`parsed_start(&self, today) -> Result<Option<Due>, String>` returning `d.due()`; delete the "start dates have no time" error), `crates/tdo/src/cli/op.rs:174-177, 297, 549` (`--start` accepts `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` through `parse_due`; help text says so), the start display in `crates/twydo-daemon/src/server/tasks.rs:79` and `server/views.rs:285` and the TUI row/details (show the time as the due does)
- Test: `crates/twydo-core/src/form.rs` tests module, `crates/tdo/tests/cli.rs`, `web/src/components/TaskRow.test.tsx` (or the existing row test file)

**Interfaces:**
- Consumes: `Change::Start(Option<Due>)` (Task 4).

- [ ] **Step 1: Write failing tests:**
  - `form::timed_start_input`: Start text `tue 10am` with today 2026-11-21 → `Change::Start(Some(Due::Time(2026-11-24T10:00)))`; the preview line reads `→ Tue 24 Nov 10:00`.
  - `form::unchanged_timed_start_writes_nothing`: a card opened on a timed-start task and saved unchanged yields no changes; its Start field text shows the time.
  - `form::bare_date_drops_start_time`: changing `2026-11-25 10:00` to `2026-11-25` → `Change::Start(Some(Due::Date(..)))`; to empty → `Change::Start(None)`.
  - `cli::edit_sets_start_time`: `tdo op edit … --start 2026-11-25T10:00` under a matrix passing `set-start.time` succeeds and the task shows the time; under the shipping matrix it is refused with the Task 5 text.
  - Web: a row for a task whose `start` text carries a time renders `Starts <text>` unchanged (the daemon formats it; guard only against the row dropping the time).
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace` and `cd web && npm test`. Expected: pass. Commit `feat(tui): type a start time in the start field`.

### Task 7: Phase B on the phone and promotion (human-in-the-loop)

**Files:**
- Create: `fixtures/set-start/twydo-roundtrip/` (states and `case.toml`)
- Modify: `compat/matrix.toml` (`set-start.time` and, if Task 5 admitted it, `complete.start-time`: all directions `pass`, evidence adds `set-start/twydo-roundtrip`), spec `## Capture results` (phase B)

**Interfaces:**
- Consumes: `cargo build --release -p tdo`; the production daemon restarted from that build with `TWYDO_NOTIFIER=off` and `tdo daemon --matrix ~/twydo-start-round/candidate-matrix.toml` (a copy of `compat/matrix.toml` with both new rows passing, not committed). Afterwards restore the mise-installed `tdo` and restart the daemon.

- [ ] **Step 1:** Back up again. Create `T1` (start 2026-11-26 10:00, no due), `T2` (start 2026-11-26 10:00, due 2026-11-28), `T3` (start 2026-11-26 10:00, due 2026-11-28 18:00), `T4` (start 2027-04-07 10:00), `T5` weekly (start 2026-11-23 09:00, due 2026-11-23 18:00) through the CLI. Sync; capture `twydo-set`.
- [ ] **Step 2:** T1 → 14:30, T2 → date only, T3 → no start; complete T5 in Twydo. Sync; capture `twydo-changed`.
- [ ] **Step 3:** Ask the user to open each T task's start on the phone and report what it shows, then edit T1's title to `T1 start edited` on the phone. Sync; capture `phone-checked`. Expected: T1 starts 26 Nov 14:30, T2 26 Nov no time, T3 no start, T4 7 Apr 10:00, T5's live task starts 30 Nov 09:00 and its copy is done; T1's start survives the phone rewrite.
- [ ] **Step 4: Go/no-go.** Any mismatch: stop, record it in Capture results, bring it to the user (spec Q5). Otherwise scan, get the user's synthetic confirmation, promote, set the rows to pass, run `python3 tools/check_evidence.py` and `cargo test --workspace`. Expected: pass. Restore the released `tdo`.
- [ ] **Step 5:** Commit `test(evidence): verify twydo-written start times on the phone`.

### Task 8: Docs and finish

**Files:**
- Modify: `docs/tasks.md:68`, `docs/tui.md:319` (start dates take a time), `docs/data-model.md:94` (`StartDate` encoding), `docs/compatibility.md` (new rows), `docs/reference/2do/06-sync.md` §2.6 (observed start-time encoding, labelled `[obs]`), wherever `tdo op` options are documented (`rg -n -- '--start' docs`), `AGENTS.md` "Current work" (start times done)

- [ ] **Step 1:** Update the pages. Run `python3 tools/check_links.py`. Expected: pass.
- [ ] **Step 2:** Run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cd web && npm test`, and commitlint from `origin/main`. Expected: pass.
- [ ] **Step 3:** Commit `docs: describe start times`.
