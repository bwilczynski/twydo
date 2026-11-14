# Alert Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set, change and remove one custom relative alert per task from the TUI card (`A`) and the CLI (`--alert`), written exactly as 2Do writes it.

**Architecture:** A phone capture round (phase A) fixes the encoding first. Then a new `Change::Alert` and its writer, a `set-alert` operation in the compatibility gate, a shared offset parser and the CLI option. Phase B is the second phone round: Twydo writes through a candidate matrix and the phone checks the result. The card field and docs follow once phase B passes.

**Tech Stack:** Rust/Cargo workspace (`twydo-core`, `tdo`, `twydo-daemon`), Python evidence tools, scoped vdirsyncer, 2Do on the iPhone (isolated test account).

**Spec:** `docs/superpowers/specs/2026-11-14-alert-editing-design.md`

## Global Constraints

- One custom relative alert per task, offset strictly before the due value, whole minutes.
- Device scope is exactly the spec's phase A steps 1–9 and phase B steps 10–12, on the isolated test account and the synthetic test list; nothing else (AGENTS.md device boundary).
- No personal data in fixtures; every staged state passes `tools/capture_fixture.py scan` and the user confirms that it is synthetic before promotion.
- Fixtures: `fixtures/set-alert/<case>/` with complete named states and `case.toml`; `python3 tools/check_evidence.py` passes.
- No `set-alert` contract is `pass` in `compat/matrix.toml` before phase B passes on the phone.
- Twydo mirrors 2Do's bytes for the custom alert and for the automatic alert's add/remove behaviour (spec Outcomes Q1/Q2/Q4). Every other VALARM and property is preserved byte for byte.
- Input grammar: empty/`none` removes; `<n><unit>` terms with `m`/`min`, `h`, `d`, `w` (combinable, e.g. `1h30m`), optional trailing `before`; days/weeks are fixed 24 h / 168 h. Display uses the largest unit that divides evenly.
- Commits pass commitlint (`type(scope): subject`, lowercase subject, ≤100 chars); no agent attribution.
- Cargo target dir under `~/.cache`, never `/tmp` (tmpfs quota).
- Tests isolate `XDG_CONFIG_HOME` and `TWYDO_SOCKET`; stop only an isolated test daemon.

## Review Focus

1. A card opened on a task with an alert and saved unchanged must write nothing. Test: `form::unchanged_alert_writes_nothing` (Task 7).
2. A task with an absolute, after-due, start-relative or second custom alert must keep its bytes and refuse with "this task has alerts Twydo can't edit". Test: `gate::set_alert_refuses_unsupported_alerts` (Task 4).
3. Changing the due date of a task with a custom alert must move the reminder with it, not drop or duplicate it. Test: `edit_alert::due_edit_keeps_custom_alert` (Task 3).
4. Undo after setting an alert must restore the exact previous bytes, including the automatic alert. Test: `store::undo_set_alert_restores_bytes` (Task 3).
5. Input like `90`, `-15m`, `0m`, `15 m`, `1d2h` must either parse to a clear meaning or fail with the examples, never panic or silently pick a unit. Test: `alertinput::rejects_ambiguous_input` (Task 5).

---

### Task 1: Phase A on the phone (human-in-the-loop)

**Files:** none in the repo; private staging `~/twydo-phase0/staging-alerts/`.

**Interfaces:**
- Produces: aliases `A1`–`A9` mapped to UIDs in `~/twydo-phase0/staging-alerts/aliases.txt`. Staged states `phone-a-before`, `phone-a-batch1` (steps 1–4, 8), `phone-a-batch2` (steps 5, 6, 9) and `phone-a-batch3` (step 7).

- [ ] **Step 1:** Ask the user to unlock `pass` (`! pass show homelab/twydo-test >/dev/null`). Back up `~/twydo-phase0/vdir` and `~/twydo-phase0/state` to `~/twydo-phase0/backup-alerts-<UTC stamp>/`. Ask the user which synthetic test list the previous rounds used and record its collection id. Run `vdirsyncer -c ~/twydo-phase0/vdirsyncer.config sync`, then capture `phone-a-before` with `capture_fixture.py capture … --operation set-due --state phone-a-before` into the staging root. Use the operation name `set-due` until Task 2 adds `set-alert`; Task 2 re-captures under the right name.
- [ ] **Step 2:** Show the user this checklist and wait until they report each batch done. All titles are synthetic, with timed dues on 2026-11-20 unless stated:

| Alias | Phone action |
|---|---|
| A1 | New task `A1 alert`, due 10:00, alert 15 min before |
| A2 | New task `A2 alert`, due 10:00, alert 4 h before |
| A3 | New task `A3 alert`, due 10:00, alert 1 day before |
| A4 | New task `A4 alert`, date-only due; try alert 1 day before and record what 2Do offers (time picker, refusal) |
| A8 | New task `A8 alert`, start 2026-11-18, due 10:00, alert 15 min before |
| — | *batch 1 ends: sync on the phone, then Twydo syncs and captures `phone-a-batch1`* |
| A5 | Open A1, change the alert to 30 min before |
| A6 | Open A2, remove the alert |
| A9 | Open A3, remove the due date |
| — | *batch 2 ends: capture `phone-a-batch2`* |
| A7 | New monthly task `A7 alert`, due 2026-11-16 09:00, alert 15 min before; sync and capture inside batch 3 as `phone-a-batch3-open`; then complete it on the phone |
| — | *batch 3 ends: capture `phone-a-batch3`* |

- [ ] **Step 3:** After each batch, sync and capture the named state with every `A*` resource listed explicitly. Ask the user to note anything 2Do showed: alert labels, whether the automatic alert is listed, and the A4 options. Record these in `~/twydo-phase0/staging-alerts/notes.md`.
- [ ] **Step 4:** Run `python3 tools/capture_fixture.py scan ~/twydo-phase0/staging-alerts` and show the output to the user for synthetic confirmation.

### Task 2: Phase A evidence case and findings

**Files:**
- Modify: `tools/evidence.py:6-10` (add `"set-alert"` to `OPERATIONS`), `tools/tests/` (the existing operation-list test, if one pins the set)
- Create: `fixtures/set-alert/phone-encodings/` (states from Task 1, `case.toml`)
- Modify: `docs/superpowers/specs/2026-11-14-alert-editing-design.md` (new section `## Capture results`)

**Interfaces:**
- Consumes: Task 1 staged states.
- Produces: the case `set-alert/phone-encodings`, which later tests read through the existing fixture helpers. The spec section `Capture results` answers Q1–Q5 and Q7 with exact property lines. It also states the custom-alert recognizer: the properties and values that tell a custom alert apart from the automatic one.

- [ ] **Step 1:** Add `"set-alert"` to `OPERATIONS`. Run `python3 -m pytest tools/tests -q`. Expected: pass.
- [ ] **Step 2:** Re-capture the four states from the staged vdirs under `--operation set-alert --case phone-encodings`. Use `--device iphone --origin device`, list every `A*` resource, set `--claim '2Do encodes a custom relative alert as …'`, and record the phone actions from Task 1 as `--action`. Copy them into `fixtures/set-alert/phone-encodings/` and fill in the `case.toml` provenance. Mark unknown client versions as unknown.
- [ ] **Step 3:** Diff the states (`diff -r`). In the spec, write `## Capture results`: the exact VALARM block 2Do writes for 15 min, 4 h and 1 day; what changed in `<2Do Meta>`/other properties; Q2 (the automatic alert on add and on remove); Q3 (date-only); Q4 (change/remove bytes); Q5 (completion copy and advanced occurrence); Q7 (start date `RELATED`, due removal). Apply the spec's Outcomes rules and state which shapes are admitted (date-only yes/no, start date yes/no, due removal drop/refuse).
- [ ] **Step 4:** **Checkpoint.** Show the user the Capture results section. If Q1 shows an encoding Twydo cannot write deterministically (for example a per-alert random UID beyond what `X-2DOAPP-*` already carries), stop here and agree how to proceed.
- [ ] **Step 5:** Run `python3 tools/check_evidence.py`. Expected: pass. Commit with the message `test(evidence): capture 2do custom alert encodings`.

### Task 3: `Change::Alert` and the alert writer

**Files:**
- Create: `crates/twydo-core/src/edit/alert.rs`
- Modify: `crates/twydo-core/src/edit/mod.rs` (`Change` enum near line 50, dispatch near line 320, `mod alert;`), `crates/twydo-core/src/store/plan.rs:79-91` and `crates/twydo-core/src/store/family.rs:128-137` (add `Change::Alert(_)` wherever `Change::Start(_)` is matched as a non-schedule field, with the same meaning as for a text edit: it does not touch checklist dates), `schema/tdo.openrpc.json` (regenerate with `tdo daemon schema > schema/tdo.openrpc.json`)
- Modify: `crates/twydo-core/src/model/task.rs` (`pub custom_alert: CustomAlert` on `Task`, computed in the parser beside `read_alarms`)
- Test: `crates/twydo-core/tests/edit_alert.rs`

**Interfaces:**
- Consumes: the Capture results recognizer (Task 2).
- Produces:
  - `Change::Alert(Option<AlertOffset>)` in `edit/mod.rs`.
  - `pub struct AlertOffset(u32)` in `edit/alert.rs`: minutes before due, `> 0`, serialized as a number of minutes, `JsonSchema`. It has `pub fn new(minutes: u32) -> Option<Self>` and `pub fn minutes(self) -> u32`.
  - `pub fn custom_alert(todo: &Component) -> CustomAlert` (also stored as `Task::custom_alert`) with `pub enum CustomAlert { None, One(AlertOffset), Unsupported }`. `Unsupported` covers absolute, after-due, start-relative (per Q7), more than one custom alert, and any VALARM that is neither automatic nor the captured custom shape.
  - `fn set_alert(todo: &mut Component, alert: Option<AlertOffset>) -> Result<(), EditError>`, private and called from the `Change` dispatch.

- [ ] **Step 1: Write failing tests** in `tests/edit_alert.rs`, each loading a resource from `fixtures/set-alert/phone-encodings`:
  - `set_alert_matches_phone_bytes_15m_4h_1d`: apply `Change::Alert(Some(15|240|1440))` to the A1/A2/A3 resource from `phone-a-before`, or to a synthetic twin with the same due if there is none. Compare to the phone's `phone-a-batch1` resource, ignoring only what the existing writer tests already ignore (`LAST-MODIFIED`, `DTSTAMP`, the `<2Do Meta>` modification stamps).
  - `change_offset_matches_phone_bytes`: A1 at 15 → 30, compared to the `phone-a-batch2` A1.
  - `remove_alert_matches_phone_bytes`: A2 → `None`, compared to the `phone-a-batch2` A2, including what happens to the automatic alert.
  - `due_edit_keeps_custom_alert`: A1 with `Change::Due(Some(Due::Time(2026-11-21 10:00)))` keeps the custom VALARM byte-identical and `custom_alert` still returns `One(15)`.
  - `custom_alert_classifies_shapes`: the automatic alert alone → `None`; the A1 batch-1 resource → `One(15)`; an absolute trigger, `-PT15M;RELATED=START` (if Q7 makes that distinct), two custom alerts, or `TRIGGER:PT15M` → `Unsupported`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test edit_alert`. Expected: FAIL (`AlertOffset` not defined).
- [ ] **Step 3:** Implement per Interfaces. The writer builds the VALARM from the captured template, the way `auto_alarm` builds its own, replaces only the existing custom VALARM in place (same position), and applies the Q2 rule to the automatic alert. On a task without a due date it returns `EditError` "an alert needs a due date". `CustomAlert` derives `Serialize`/`Deserialize`/`JsonSchema`, like `Alarm`.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: pass. Regenerate the schema; `cargo test -p twydo-daemon` passes. An additive variant must pass the released-schema compatibility test; if that test rejects it, report the failure rather than editing `released.openrpc.json`.
- [ ] **Step 5:** Commit with the message `feat(core): add a custom alert edit`.

### Task 4: The `set-alert` operation in the gate

**Files:**
- Modify: `crates/twydo-core/src/compat/contracts.rs` (`OperationId::SetAlert` / `"set-alert"`; `SetAlertTask => SetAlert, "set-alert.task", "setting an alert"`), `crates/twydo-core/src/gate.rs` (the `Op::Edit` dispatch near line 1090 maps `Change::Alert(_)` to `C::SetAlertTask`, plus a new admission function), `compat/matrix.toml` (row `set-alert.task`, `iphone_baseline = "pass"`, `twydo_to_iphone` unverified, evidence `["set-alert/phone-encodings"]`)
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/store.rs`

**Interfaces:**
- Consumes: `custom_alert`, `CustomAlert`, `Change::Alert` (Task 3).
- Produces: `fn alert_shape(snap: &Snapshot, i: usize, changes: &[Change]) -> Result<(), Blocked>` in `gate.rs`, called for any edit containing `Change::Alert`. The refusal texts are fixed: `"an alert needs a due date"`, `"this task has alerts Twydo can't edit"`, `"checklist items have no alerts"`, plus `"an alert on a task with a date but no time is not supported"` / `"an alert on a task with a start date is not supported"` when Task 2 excludes those shapes.

- [ ] **Step 1: Write failing tests:**
  - `set_alert_admits_timed_task`: open timed task, none or one custom alert, any repeat class → admitted under a matrix with `set-alert.task` passing; refused as unverified under the shipping matrix.
  - `set_alert_refuses_unsupported_alerts`: absolute, after-due, two custom alerts → `"this task has alerts Twydo can't edit"`, and the resource is unchanged.
  - `set_alert_refuses_without_due`: `"an alert needs a due date"`. A save that combines `Change::Due(None)` with `Change::Alert(Some)` is refused too, because the due date is checked after the changes apply (the `due_after` helper).
  - `set_alert_refuses_checklist_item`: `"checklist items have no alerts"`.
  - Date-only and start-date admission or refusal, as Task 2 decided.
  - `due_removal_with_custom_alert`: mirrors Q7 (refuse with a reason, or drop the alert in the writer, as Task 2 decided).
  - `undo_set_alert_restores_bytes` in `tests/store.rs`, using `tests/common/store.rs` and a matrix that passes `set-alert.task`: set an alert, undo, and assert the resource bytes equal the original; redo, and assert they equal the post-set bytes.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate set_alert`. Expected: FAIL.
- [ ] **Step 3:** Implement the contract, the dispatch and `alert_shape`. Leave the existing `captured_automatic_alarm` refusals unchanged (spec: "Other operations are unchanged").
- [ ] **Step 4:** Run `cargo test -p twydo-core` and `python3 tools/check_evidence.py`. Expected: pass.
- [ ] **Step 5:** Commit with the message `feat(gate): admit custom alert edits behind set-alert.task`.

### Task 5: Alert input parser and the CLI option

**Files:**
- Create: `crates/twydo-core/src/alertinput.rs` (beside `dateinput.rs`/`repeatinput.rs`; register it in `lib.rs`)
- Modify: `crates/tdo/src/cli/op.rs:160-191` (`EditArgs`) and the place that turns `EditArgs` into `Vec<Change>`
- Test: `crates/twydo-core/tests/alertinput.rs`, `crates/tdo/tests/cli.rs`

**Interfaces:**
- Produces:
  - `pub fn parse(input: &str) -> Result<Option<AlertOffset>, String>`. `Ok(None)` means remove.
  - `pub fn format(offset: AlertOffset) -> String`, using the largest unit that divides evenly, with no spaces: `15m`, `4h`, `1d`, `2w`, `90m`.
  - `pub const EXAMPLES: &str = "15m, 4h, 1d, 2w, 1h30m, none"`.
  - CLI: `--alert <TEXT>` (conflicts with `--no-alert`) and `--no-alert`.

- [ ] **Step 1: Write failing tests** in `tests/alertinput.rs`:
  - `parses_units`: `15m`→15, `15min`→15, `4h`→240, `1d`→1440, `2w`→20160, `1h30m`→90, `1d2h`→1560, `15m before`→15, `4H`→240.
  - `removes`: `""`, `"none"`, `"  "` → `Ok(None)`.
  - `rejects_ambiguous_input`: `90`, `-15m`, `0m`, `15 m`, `15x`, `m`, `1.5h`, `15m after` → `Err` whose text contains `EXAMPLES`.
  - `format_round_trips`: for 15, 30, 90, 240, 1440, 1560, 10080, `parse(&format(x)) == x`. `format(1440) == "1d"` and `format(1560) == "26h"`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test alertinput`. Expected: FAIL.
- [ ] **Step 3:** Implement. `15 m` is rejected (a unit must follow its number directly); `before` is the only trailing word allowed.
- [ ] **Step 4:** Add CLI tests to `crates/tdo/tests/cli.rs` that follow the existing `--repeat` tests: `edit_sets_alert` (`--alert 15m`, then `tdo show`/export shows the custom alert) and `edit_rejects_bad_alert` (exit 2, the message lists the examples). Run the isolated daemon with a test matrix that passes `set-alert.task`, following how the existing CLI tests pass `--matrix`.
- [ ] **Step 5:** Run `cargo test -p twydo-core --test alertinput` and `cargo test -p tdo --test cli`. Expected: pass. Commit with the message `feat(cli): set and remove an alert with --alert`.

### Task 6: Phase B on the phone and promotion (human-in-the-loop)

**Files:**
- Create: `fixtures/set-alert/twydo-roundtrip/` (states and `case.toml`)
- Modify: `compat/matrix.toml` (`set-alert.task`: `twydo_to_iphone = "pass"`, `iphone_to_twydo = "pass"`, evidence adds `set-alert/twydo-roundtrip`), `docs/superpowers/specs/2026-11-14-alert-editing-design.md` (`Capture results`: phase B)

**Interfaces:**
- Consumes: a release build of this branch (`cargo build --release -p tdo`) and the test daemon at `~/twydo-phase0/xdg`, started with `tdo daemon --matrix <candidate>`. The candidate is a copy of `compat/matrix.toml` with `set-alert.task` passing, kept in the staging directory and not committed.

- [ ] **Step 1:** Create synthetic tasks `B1`–`B4` through the CLI: timed dues 2026-11-21 10:00, plus one date-only due if Task 2 admits it. Set `B1 --alert 15m`, `B2 --alert 4h`, `B3 --alert 1d`, and `B4 --alert 1d` (date-only, if admitted). Sync, then capture `twydo-set`.
- [ ] **Step 2:** Change B1 to `--alert 45m` and run `--no-alert` on B2. Sync, then capture `twydo-changed`.
- [ ] **Step 3:** Ask the user to open each B task's alert editor on the phone and report the alert it shows. Ask them to edit B1's title to `B1 alert edited` on the phone. Sync, then capture `phone-checked`. Expected: the phone shows 45 min, none, 1 day and (B4) 1 day, with no duplicate alerts. B1's custom VALARM survives the phone's rewrite.
- [ ] **Step 4:** **Go/no-go.** If any expectation fails, stop, write the failure into `Capture results`, and bring it to the user (spec Outcome Q6). Otherwise scan, have the user confirm the content is synthetic, promote the case, set the matrix row to pass, and run `python3 tools/check_evidence.py` and `cargo test --workspace`. Expected: pass.
- [ ] **Step 5:** Commit with the message `test(evidence): verify twydo-written alerts on the phone`.

### Task 7: The Alert field on the edit card and `A`

**Files:**
- Modify: `crates/tdo/src/form.rs` (`Field::Alert` after `Field::Repeat` in both `fields()` lists, but not for checklist items; `alert: String` text; `changes()` emits `Change::Alert` only when the parsed value differs from the task's current one), `crates/tdo/src/ui/card.rs` (the row label `Alert`, placeholder `EXAMPLES`, a read-only display for `CustomAlert::Unsupported`), `crates/tdo/src/app.rs:909` (`'A' => self.open_form(Field::Alert)`), `crates/tdo/src/ui/mod.rs:286` (help: `("A", "card on the Alert field", false)`)
- Test: `crates/tdo/tests/form.rs`, `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `custom_alert`, `CustomAlert` (Task 3), `alertinput::{parse, format, EXAMPLES}` (Task 5). `EditForm::open` reads `task.custom_alert` (Task 3).

- [ ] **Step 1: Write failing tests:**
  - `form::unchanged_alert_writes_nothing`: open a task with `One(15)`; `changes()` is empty.
  - `form::alert_field_sets_and_clears`: typing `4h` gives `[Change::Alert(Some(240))]`, and typing `none` gives `[Change::Alert(None)]`.
  - `form::alert_hidden_for_checklist_items`.
  - `form::unsupported_alert_is_read_only`: typing on the field leaves the text unchanged and sets the message `"this task has alerts Twydo can't edit"`.
  - `app_keys::capital_a_opens_card_on_alert`.
  - `ui_render`: a card snapshot with the Alert row showing `15m`.
- [ ] **Step 2:** Run `cargo test -p tdo --test form --test app_keys --test ui_render`. Expected: FAIL.
- [ ] **Step 3:** Implement. A combined save of Alert and other fields stays one `Op::Save`, as today.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: pass. Commit with the message `feat(tui): edit a task's alert on the card with A`.

### Task 8: Notifier pin, docs, finish

**Files:**
- Test: `crates/twydo-core/tests/remind.rs` (`plans_twydo_written_alert`: the B1 `phone-checked` resource plans one reminder at due − 45 min and one at the due time)
- Modify: `docs/tasks.md` (Alerts section: drop "not available", describe the input grammar, supported shapes and refusals, and keep the existing narrow refusals), `docs/tui.md` (keys table: `A`; card fields table: Alert; an alert input table beside the date/repeat ones; drop "alerts" from the not-yet-available sentence near line 447), `docs/compatibility.md` (a `set-alert.task` row and a paragraph on the evidence; remove "native alert editing" from the unverified list), `docs/reference/evidence.md` (operation table row Alerts; record the alerts round as complete, not to be repeated), `docs/superpowers/plans/2026-11-04-plan-8-editing-completeness.md:153,303` (mark custom alert editing done), `AGENTS.md` (one line under Current work)

- [ ] **Step 1:** Write the remind test and run `cargo test -p twydo-core --test remind`. Expected: pass. If it fails, it exposes a notifier gap; fix it under systematic debugging.
- [ ] **Step 2:** Update the docs and run `python3 tools/check_links.py`. Expected: pass.
- [ ] **Step 3:** Run `mise run test` and commitlint (`npx --yes -p @commitlint/cli -p @commitlint/config-conventional commitlint --from=origin/main --to=HEAD`). Expected: both pass.
- [ ] **Step 4:** Commit with the message `docs: describe alert editing`.
