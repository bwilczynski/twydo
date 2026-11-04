# Plan 7: adding and reshaping tasks (phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add tasks, projects, checklists and their items inline in the TUI, edit them in an
in-place card, change their type, and set any of 2Do's repeat rules, all written exactly as 2Do
on the iPhone writes them.

**Architecture:** Phone captures come first (Task 1): every new encoding (repeat classes,
parents, items, conversions) is copied from what 2Do wrote, then byte-compared in core tests.
`twydo-core` gains a repeat-text parser, new create fields, two new `Change`s and a grouped
save; the gate learns the new cases. The TUI replaces the centered form with an in-place card,
adds the new row (`o`/`O`/`a`) and `ct`/`cp`/`cc`. A second phone session (Task 13) closes the
`twydo_to_iphone` columns.

**Tech Stack:** Rust 2024, ratatui/crossterm, existing `twydo-core` ical/edit/store/gate
modules, `mise run lint` and `mise run test`, Python 3 tools for capture.

**Spec:** [`docs/superpowers/specs/2026-11-03-tui-add-and-structure-design.md`](../specs/2026-11-03-tui-add-and-structure-design.md)
(phase 1 = its §3, §4, §7, §8). The main spec
[`2026-10-31-terminal-tasks-design.md`](../specs/2026-10-31-terminal-tasks-design.md) governs the
rest (compatibility gate, preservation, journal).

## Global Constraints

- No personal data in the repository (AGENTS.md): fixtures come from the **test account**
  with synthetic single-letter titles; check `git diff --staged` before each commit.
- Ask the user before anything that touches production 2Do data, the NAS or the iPhone. The
  test setup is `~/twydo-phase0` (XDG `~/twydo-phase0/xdg`, vdirsyncer pair `twydo`); never use
  the production pair `twydo_prod`.
- Verified mode refuses every write whose matrix row or rule verdict is not `pass`, with a
  reason in the form `… is not verified yet`.
- Commit messages: conventional commits (release-plz reads them), **no agent attribution**
  (no `Co-Authored-By`, no "Generated with").
- When behaviour changes, update the matching `docs/` page in the same change;
  `python3 tools/check_links.py` must pass.
- Each task ends green: `mise run lint` and `mise run test`.
- `Esc` always cancels (on the new row, in the card, on questions); nothing is written.
- Key spellings and messages are fixed by the spec: `o`, `O`, `a`, `Tab`, `i`, `r`, `ct`/`cp`/`cc`;
  `New task in <list> · Enter save  Tab more  Esc cancel`;
  `Add items: make it a (p)roject or (c)hecklist?`;
  `Items lose their dates and repeats. Convert? (y/n)`; `Added "<title>" to <list>`. (The
  `Convert? (y/n)` question is superseded by the Plan 7 capture,
  `docs/reference/phase0/plan7-capture.md`: the iPhone cannot convert a project with items, so
  the gate refuses it permanently and nothing asks.)

## Review Focus

1. **A new task in a view it does not belong to** (`o` in Scheduled or Done): the task must still
   be written to the default list and the status line must say `Added "…" to Inbox`, cursor
   unmoved. Test in Task 10.
2. **A sync reloads the vdir while a new row or card is open:** the typed text must survive and
   the save must go against the snapshot the row opened on (stale → refused with the existing
   "changed on disk" message, text kept). Test in Task 10 and Task 9.
3. **Repeat text that parses but is not verified** (`2nd tue` in verified mode): the preview shows
   the rule, saving is refused with `months-nth-weekday repeats are not verified yet`, the card
   stays open with the text. Test in Task 6.
4. **A card save that mixes an allowed and a refused change** (title edit + unverified type
   change): nothing is written, one message names the refused part. Test in Task 7.
5. **`Enter` chain when the parent disappears** (deleted on the phone mid-chain): the next save is
   refused with `no task with UID …` and the row closes; earlier items stay. Test in Task 10.

---

### Task 1: Capture what 2Do writes (with the user, test phone)

**Files:**
- Create: `docs/reference/phase0/plan7-capture.md` (rounds, then results)
- Create: `fixtures/phase0/v1/plan7-list1/<round>/` (captured `.ics`, via the existing capture tool
  used in `gap-session.md`)
- Modify: `compat/matrix.toml` (new rows, `iphone_baseline` and `iphone_to_twydo` columns)
- Modify: `docs/reference/2do/01-features.md` (§1 conversions, §6 "Every Day": turn `[?]` into `[obs]`)

**Interfaces:**
- Produces: fixture directories named in this task, used as byte references by Tasks 3–6.
  Matrix cases `daily`, `weekly` (ops create/edit/complete/reopen/move/delete) and op `convert` with
  cases `task-to-checklist`, `task-to-project`, `checklist-to-project`, `project-to-checklist`,
  `parent-to-task`; every new column `unverified` unless this session passes it.

- [ ] **Step 1: Write the rounds** in `plan7-capture.md`, same format as `gap-session.md`
  (👤/🤖 markers, single-letter titles, `Twydo test` list). 👤 rounds on the phone:
  - 01 create, one task per repeat kind with the 2Do picker: Every Day preset, Daily (custom
    every 1 day), every 3 days, Weekly, every 2 weeks, Mon+Thu, Work days, Monthly on the 31st,
    last day of month, 2nd Tuesday, last weekday, 1st weekend day, Yearly on 29 Feb, from
    completion (monthly), end after 3, end on a date. Each with a due date.
  - 02 set a repeat on a task **without** a due date (does 2Do fill one in?).
  - 03 conversions: task→checklist, task→project, checklist with 2 items→project, project with
    2 dated items→checklist (record the warning), project with 2 items→task (record the dialog and
    where the items end up), empty checklist→task.
  - 04 add an item to an existing checklist and to an existing project.
  🤖 after each round: sync, capture, `tdo export-json` to compare with what the phone shows.
- [ ] **Step 2: Ask the user to approve the rounds**, then run them together. Do not start
  before a yes.
- [ ] **Step 3: Record results** in `plan7-capture.md`: for each kind the exact `RRULE` value and
  the JSON `RecurrenceType`/`RecurrenceValue`/`RecurrenceFrom`/`RecurrenceEnd*` values; the
  Every Day encoding; whether a due date was filled; each conversion's changed lines; the
  parent→task behaviour. Fill `iphone_baseline` for the new matrix rows and run
  `cargo test -p twydo-core --test gate` to confirm the matrix still loads.
- [ ] **Step 4: Commit**

```bash
git add docs/reference/phase0/plan7-capture.md fixtures/phase0/v1/plan7-list1 compat/matrix.toml docs/reference/2do/01-features.md
git commit -m "docs(phase0): capture 2Do's repeat kinds, conversions and new items"
```

### Task 2: Read every captured shape correctly

**Files:**
- Modify: `crates/twydo-core/src/model/repeat.rs` (only if a capture reads wrong)
- Test: `crates/twydo-core/tests/repeat.rs`

**Interfaces:**
- Consumes: Task 1 fixtures.
- Produces: confirmed `rule_class` and `describe` output per capture (used as preview text in Task 3).

- [ ] **Step 1: Write the failing test** `plan7_captures_read_as_2do_shows_them`: for each
  01-create capture, `rule_class(&rrule, due, from_completion)` equals the class from 01 §6
  (`days`, `weeks`, `weekdays-set`, `months-day-29-31`, `months-last-day`, `months-nth-weekday`,
  `months-nth-workday`, `months-nth-weekend-day`, `years-feb-29`, with `+from-completion`,
  `+count`, `+until`) and `describe(..)` equals 2Do's label recorded in Task 1.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test repeat plan7` — expected FAIL only where the
  reader disagrees (if it passes outright, keep the test as the regression guard).
- [ ] **Step 3: Fix the reader** where it disagrees; 2Do's label wins.
- [ ] **Step 4: Run** the same command — PASS. Set `iphone_to_twydo` = `pass` on each row the
  test covers, and each rule's `preserve` = `pass` once `ical_roundtrip` shows the capture
  round-trips byte-equal (add those files to its fixture list).
- [ ] **Step 5: Commit** `fix(core): read every 2Do repeat kind as 2Do shows it` (or `test(core): …`).

### Task 3: Repeat text input (`twydo_core::repeatinput`) and `work_days`

**Files:**
- Create: `crates/twydo-core/src/repeatinput.rs`
- Modify: `crates/twydo-core/src/lib.rs`, `crates/twydo-core/src/config.rs`
- Test: `crates/twydo-core/tests/repeatinput.rs`, `crates/twydo-core/tests/config.rs`

**Interfaces:**
- Consumes: `model::repeat::{RRule, Freq, End, describe, rule_class}`, `dateinput::parse` (for `until <date>`).
- Produces:
  - `pub struct RepeatSpec { pub rule: RRule, pub from_completion: bool }` (derive Debug, Clone, PartialEq, Eq)
  - `pub fn parse(input: &str, today: NaiveDate, work_days: &[Weekday]) -> Result<Option<RepeatSpec>, String>` (`Ok(None)` = no repeat)
  - `pub fn describe(spec: &RepeatSpec, anchor: Option<NaiveDate>) -> String` (preview; wraps `repeat::describe`, adds ` · from due date` / ` · after done`)
  - `pub fn to_text(rrule: &str, from_completion: bool) -> Option<String>` (prefill; `None` for a rule the syntax cannot express)
  - `pub const EXAMPLES: &str = "daily · weekly · 2w · monthly · mon thu · workdays · last day · 2nd tue · yearly";`
  - `Config::work_days: Vec<Weekday>`, TOML `work_days = ["mon","tue","wed","thu","fri"]` (default Mon–Fri)

- [ ] **Step 1: Write the failing tests** `parses_every_row_of_the_spec_table` (table-driven, one
  case per row of spec §3.5 and each modifier: `none` and `""` → `Ok(None)`; `daily`/`every day`/`3d`/
  `every 3 days`; `weekly`/`biweekly`/`2w`/`every 2 weeks`; `monthly`/`3m`; `yearly`/`2y`;
  `mon thu`/`mon,thu`/`every mon and thu`; `workdays` → BYDAY = the given work days; `last day`,
  `last day every 2m`; `2nd tue`, `last fri`, `1st sun`; `1st weekday`, `last weekday`;
  `1st weekend day`, `last weekend day`; `monthly after done`; `weekly until 2027-06-01`;
  `monthly 10 times`), each asserting the `RRule` fields and the class the Task 1 capture has.
  `daily` produces whatever the Every Day capture showed. `rejects_with_reason`: `every`,
  `6th tue`, `0d`, `monthly 15th` → `Err` naming the problem. `to_text_round_trips`: for every
  table row, `parse(to_text(x))` equals `x`. `describe_matches_2do_labels` against Task 2's labels.
  Config test `work_days_default_and_custom`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test repeatinput` — FAIL (module missing).
- [ ] **Step 3: Implement** `repeatinput.rs` (lowercase, split on whitespace/commas; modifiers
  stripped from the end first) and the config key (reuse the existing weekday deserializer of
  `week_start`).
- [ ] **Step 4: Run** `cargo test -p twydo-core --test repeatinput --test config` — PASS.
- [ ] **Step 5: Commit** `feat(core): parse typed repeat rules`.

### Task 4: Write a repeat rule into a resource (`Change::Repeat`)

**Files:**
- Modify: `crates/twydo-core/src/edit/mod.rs` (new `Change::Repeat`, `set_repeat`)
- Create: `crates/twydo-core/src/edit/repeat.rs` (2Do codes)
- Test: `crates/twydo-core/tests/edit_fields.rs`

**Interfaces:**
- Consumes: `repeatinput::RepeatSpec`, `MetaJson`.
- Produces:
  - `Change::Repeat(Option<RepeatSpec>)` (`None` removes the repeat)
  - `pub fn twodo_codes(r: &RRule) -> (i64, i64)` in `edit/repeat.rs`: `(RecurrenceType, RecurrenceValue)`
    per 01 §6 "Exact encoding" (bitmask Sun 1 … Mon 64; `0x100`–`0x10D`)
  - `fn set_repeat(todo: &mut Component, spec: Option<&RepeatSpec>, due_fill: Option<Due>) -> Result<(), EditError>`
    (private; `due_fill` per Task 1's finding on repeats without a due date)

- [ ] **Step 1: Write the failing tests** `set_repeat_matches_phone_edit_<class>` for each class
  captured in Task 1: start from a plain-task capture, apply `Change::Repeat(Some(spec))`, compare
  `sorted(lines(..))` with the phone's capture excluding stamps and UID (pattern of
  `edit_create.rs`). `remove_repeat_matches_phone` (RRULE gone, JSON codes back to 0, `RUID` as the
  phone left it). `codes_follow_the_2do_table` (one assert per table row). `repeat_without_due`
  matches Task 1 round 02.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test edit_fields repeat` — FAIL.
- [ ] **Step 3: Implement.** `RRULE` goes after `DUE` (as captured); JSON keys set with
  `MetaJson::set_i64`, so unchanged keys keep their bytes; `RUID` = own UID while repeating.
- [ ] **Step 4: Run** the tests — PASS.
- [ ] **Step 5: Commit** `feat(core): set and remove repeat rules as 2Do writes them`.

### Task 5: Create with type, parent and repeat

**Files:**
- Modify: `crates/twydo-core/src/edit/mod.rs` (`NewTask`), `crates/twydo-core/src/edit/create.rs`
- Modify: every `NewTask { .. }` literal (`cli/op.rs`, tests): add the new fields
- Test: `crates/twydo-core/tests/edit_create.rs`

**Interfaces:**
- Produces: `NewTask { collection, summary, description, due, start, kind: TaskKind, parent: Option<String>, repeat: Option<RepeatSpec> }`;
  `impl NewTask { pub fn plain(collection: &str, summary: &str) -> NewTask }` (Task kind, no dates).

- [ ] **Step 1: Write the failing tests** against `plan6-list1/01-create` and Task 1 captures,
  same comparison as the existing create tests: `creates_a_project_like_the_phone` (UID `09c49c…`,
  JSON order `RecurrenceValue, RecurrenceEndRepetitionsOrig, uid, RecurrenceEndRepetitions,
  TaskType, TaskDuration, …` with `TaskType` 2), `creates_a_checklist_like_the_phone` (`a8dcf9…`,
  `TaskType` 1), `creates_a_checklist_item_like_the_phone` (`28ae80…`: `RELATED-TO` after `UID`,
  `parentuid` before `RUID`), `creates_a_project_item_like_the_phone` (`e66881…`),
  `creates_a_repeating_task_like_the_phone_<class>` (Task 1 round 01),
  `creates_a_recurring_project_like_the_phone` (`8f603c…`).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test edit_create` — FAIL.
- [ ] **Step 3: Implement** in `new_task`; reuse `set_repeat` from Task 4.
- [ ] **Step 4: Run** — PASS; `cargo test --workspace` still green.
- [ ] **Step 5: Commit** `feat(core): create projects, checklists, items and repeating tasks`.

### Task 6: Change type (`Change::Kind`) and its family writes

**Files:**
- Modify: `crates/twydo-core/src/edit/mod.rs` (`Change::Kind`), `crates/twydo-core/src/store/mod.rs`,
  `crates/twydo-core/src/store/family.rs`
- Test: `crates/twydo-core/tests/edit_fields.rs`, `crates/twydo-core/tests/store_family.rs`

**Interfaces:**
- Consumes: `edit::family::drop_member_dates` (existing).
- Produces: `Change::Kind(TaskKind)`; store plans an `Edit` with `Change::Kind(Checklist)` on a
  project with items as a family write (items' dates and repeats dropped, as round 03 showed).

- [ ] **Step 1: Write the failing tests** one per conversion captured in Task 1 round 03,
  comparing every touched file with the phone's version: `task_to_checklist`, `task_to_project`,
  `checklist_to_project`, `project_to_checklist_drops_item_dates` (family: one journal entry
  writes parent + items), `empty_parent_to_task`. If round 03 showed a safe 2Do behaviour for a
  parent with items → task, add `parent_with_items_to_task` matching it; else
  `parent_with_items_to_task_is_refused` with
  `turning a project or checklist with items into a task is not verified yet`.
- [ ] **Step 2: Run** `cargo test -p twydo-core --test edit_fields kind --test store_family kind` — FAIL.
- [ ] **Step 3: Implement.** Tokens-format metadata (no JSON) is refused with
  `changing the type of this task is not supported` (since the final review:
  `this task's 2Do metadata format is read-only`, the one message for every write that needs
  JSON metadata, `set_repeat` included).
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(core): convert between task, project and checklist`.

### Task 7: Gate rules for the new writes, and the grouped save

**Files:**
- Modify: `crates/twydo-core/src/gate.rs`, `crates/twydo-core/src/edit/mod.rs` (`Op::Save`),
  `crates/twydo-core/src/store/mod.rs`
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/store.rs`

**Interfaces:**
- Produces:
  - `Op::Save { uid: String, changes: Vec<Change>, to: Option<String> }`: one journal entry
    (kind `"save"`) = the edit's writes, then the move's writes planned on the edited content; a
    path written twice keeps the first `before` and the last `after`. One undo step.
  - Gate: `own_case` returns `daily` (class `days`) and `weekly` (`weeks`, `weekdays-set`) besides
    `monthly`/`yearly`; `+from-completion` uses the `from-completion` case instead of refusing.
    Creating with or setting a repeat of class C needs C's `preserve` and `edit` = pass
    (else `C repeats are not verified yet`). `Create` with `parent`: parent must be a declared
    checklist or project, not an item; family case row `create`; a checklist item with a due,
    start or repeat → `checklist items take their date from the checklist`. `Change::Kind` → op
    `convert`, case from the before/after kinds. `Op::Save` checks the edit part and the move
    part; the first refusal wins and nothing is written.

- [ ] **Step 1: Write the failing tests** (gate.rs, verified mode, matrix built with
  `common::matrix`): `create_item_needs_family_create_row`,
  `create_item_under_plain_task_is_refused` (`subtasks of a plain task are not verified yet`),
  `checklist_item_with_date_is_refused`, `new_repeat_needs_rule_edit_verdict`
  (`months-nth-weekday repeats are not verified yet` — Review Focus 3),
  `weekly_task_has_case_weekly`, `convert_needs_convert_row` (`… is not verified yet`),
  `save_with_one_refused_part_writes_nothing` (store.rs: title + unverified kind change → `Blocked`,
  vdir unchanged, journal empty — Review Focus 4), `save_with_move_is_one_undo_step`
  (store.rs: title + move → one entry; `undo_last` restores both files).
- [ ] **Step 2: Run** `cargo test -p twydo-core --test gate --test store` — FAIL.
- [ ] **Step 3: Implement**; extend the module comment's rule list in `gate.rs`.
- [ ] **Step 4: Run** — PASS; whole workspace green.
- [ ] **Step 5: Commit** `feat(core): gate new items, repeats and conversions; grouped save`.

### Task 8: `tdo op` for the new operations

**Files:**
- Modify: `crates/tdo/src/cli/op.rs`
- Test: `crates/tdo/tests/cli_op.rs`; docs: `docs/compatibility.md` (the `tdo op add creates plain tasks only` paragraph)

**Interfaces:**
- Produces: `tdo op add … [--kind task|project|checklist] [--parent UID] [--repeat TEXT]`;
  `tdo op edit UID … [--repeat TEXT] [--no-repeat] [--kind KIND]`. `--repeat` goes through
  `repeatinput::parse` with today and the config's `work_days`; a parse error exits 2.

- [ ] **Step 1: Write the failing tests** `add_item_under_checklist`, `add_with_repeat`,
  `edit_kind`, `edit_repeat_bad_text_exits_2` (stderr names the problem).
- [ ] **Step 2: Run** `cargo test -p tdo --test cli_op` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(cli): add items, kinds and repeats with tdo op`.

### Task 9: The in-place card (replaces the centered form)

**Files:**
- Modify: `crates/tdo/src/form.rs` (fields List, Type, full Repeat; new-task mode)
- Create: `crates/tdo/src/ui/card.rs`; Delete: `crates/tdo/src/ui/form.rs` (fallback moves into card.rs)
- Modify: `crates/tdo/src/ui/tasks.rs` (reserve rows under the selected task), `crates/tdo/src/ui/mod.rs`,
  `crates/tdo/src/app.rs` (`open_form`, `form_key`, `r` on any task, list picker from the card)
- Test: `crates/tdo/tests/form.rs`, `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `repeatinput::{parse, describe, to_text, EXAMPLES}`, `Op::Save`.
- Produces:
  - `Field::{Title, List, Type, Due, Start, Repeat, Notes}`; `EditForm::fields()` returns only
    `Title, Notes` for a checklist item, and omits `List` for any item.
  - `EditForm::new_task(target: NewTarget, title: String, tz: Tz) -> EditForm` and
    `EditForm::save(&self, today, work_days) -> Result<Op, String>` (`Op::Create` for a new task;
    `Op::Save` for an existing one; `Err("no changes")` when nothing changed).
  - `pub struct NewTarget { pub collection: String, pub parent: Option<String>, pub due_today: bool }` (in `form.rs`; Task 10 builds it).
  - `FormAction::PickList` (Enter on List) → the existing `ListPicker`, whose choice sets the field instead of writing.

- [ ] **Step 1: Write the failing tests:** form.rs — `repeat_field_prefills_rule_text`,
  `repeat_preview_and_examples` (empty field shows `EXAMPLES`), `type_field_cycles_with_space_and_hl`,
  `checklist_item_has_title_and_notes_only`, `save_builds_one_op_with_list_change`.
  app_keys.rs — `r_opens_repeat_on_a_plain_task`, `esc_in_card_writes_nothing`,
  `refused_save_keeps_card_and_text`, `sync_reload_keeps_card_text` (Review Focus 2).
  ui_render.rs — `card_renders_in_place_below_row` (rows below shift; Details still drawn at
  120 cols), `card_falls_back_to_overlay_under_60_cols`.
- [ ] **Step 2: Run** `cargo test -p tdo` — FAIL.
- [ ] **Step 3: Implement.** The card's height decides the scroll offset so it is fully visible.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(tui): edit tasks in an in-place card with list, type and repeat`.

### Task 10: The new row (`o`, `O`, `a`, the `Enter` chain)

**Files:**
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/ui/tasks.rs`, `crates/tdo/src/ui/status.rs`
- Test: `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `NewTarget`, `EditForm::new_task`, `NewTask::plain`, `Config::default_list`.
- Produces: `App::new_row: Option<NewRow>` with `pub struct NewRow { pub target: NewTarget, pub title: String, pub below: bool, pub anchor: Option<String> /* UID */ }`;
  `fn default_list(snap: &Snapshot) -> String` (config `default_list`, else the list named
  Inbox, else the first list).

- [ ] **Step 1: Write the failing tests:** `o_in_list_targets_that_list`,
  `o_in_today_is_due_today_in_default_list`, `o_in_scheduled_adds_to_inbox_and_says_so`
  (message `Added "x" to Inbox`, cursor unmoved — Review Focus 1), `o_on_item_targets_parent`,
  `O_opens_above`, `enter_in_parent_chains_next_item`, `enter_on_empty_row_closes`,
  `esc_discards_row_keeps_saved_items`, `a_on_parent_opens_row_at_end_and_expands`,
  `a_on_plain_task_asks_p_or_c` (then `c` → `Op::Save` with `Change::Kind(Checklist)` and the row
  opens after it finishes; other key cancels), `tab_moves_row_into_card_with_title`,
  `chain_after_parent_deleted_closes_row` (Review Focus 5), `status_line_names_target`.
  ui_render.rs — `new_row_draws_under_cursor`.
- [ ] **Step 2: Run** `cargo test -p tdo --test app_keys --test ui_render` — FAIL.
- [ ] **Step 3: Implement.** The row keeps the snapshot it opened on (`form_base` pattern) so
  a reload does not lose text; after a successful save the cursor follows the new UID.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(tui): add tasks and items inline with o, O and a`.

### Task 11: `ct` / `cp` / `cc`

**Files:**
- Modify: `crates/tdo/src/app.rs`
- Test: `crates/tdo/tests/app_keys.rs`

- [ ] **Step 1: Write the failing tests:** `ct_cp_cc_emit_kind_change`, `cp_on_project_does_nothing`,
  `cc_on_project_with_dated_items_asks_first` (message `Items lose their dates and repeats. Convert? (y/n)`;
  `y` writes, `n`/`Esc` writes nothing), `c_then_other_key_cancels`, `c_is_ignored_under_overlay`.
- [ ] **Step 2: Run** `cargo test -p tdo --test app_keys c` — FAIL.
- [ ] **Step 3: Implement** `c` as a pending prefix like `d`/`g`.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(tui): change a task's type with ct, cp and cc`.

### Task 12: Docs and help

**Files:**
- Modify: `docs/tui.md` (keys, the new row, the card, `Esc`), `docs/tasks.md` (types, conversions,
  repeat syntax table from spec §3.5), `docs/config.md` (`work_days`; `default_list` is no longer "reserved"),
  `crates/tdo/src/ui/mod.rs` (`?` help), `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md`
  (§4 decision 5, §8 keymap, §10 step 4, as listed in the design's "Changes to the main spec"),
  `AGENTS.md` ("Done so far": Plan 7 phase 1)
- Test: `crates/tdo/tests/ui_render.rs` (`help_lists_new_keys`)

- [ ] **Step 1: Write the failing test** `help_lists_new_keys` (`o`, `O`, `a`, `ct`, `cp`, `cc` present).
- [ ] **Step 2: Run** — FAIL. **Step 3: Update help and docs.** **Step 4: Run** `mise run test` and
  `python3 tools/check_links.py` — PASS.
- [ ] **Step 5: Commit** `docs: adding, editing and reshaping tasks`.

### Task 13: Verify Twydo's writes on the phone (with the user)

**Files:**
- Modify: `docs/reference/phase0/plan7-capture.md` (session B), `compat/matrix.toml`
  (`twydo_to_iphone`, rule `edit`/`execute`), `fixtures/phase0/v1/plan7-twydo-list1/`

- [ ] **Step 1: Write session B rounds** (🤖 writes with `tdo op` in `test` mode, 👤 checks on
  the phone, then edits/completes there): create a project and a checklist with items; add an
  item to each; one repeating task per class and modifier; each conversion; set, change and
  remove a repeat; complete one of each repeating class on the phone and in Twydo.
- [ ] **Step 2: Ask the user to approve**, then run together on the test account.
- [ ] **Step 3: Record results**; set each passed column; leave the rest `unverified` with a
  reason in `notes`. Fix any byte mismatch in Tasks 3–6's code with a test from the new capture.
- [ ] **Step 4: Run** `mise run test` — PASS (matrix-driven gate tests follow the new verdicts).
- [ ] **Step 5: Commit** `docs(phase0): verify Twydo's new writes on the iPhone`.
- [ ] **Step 6: Confirm with the user** which daily shapes (plain, project, checklist, monthly
  on a day, yearly; items; conversions they use) now pass, before phase 1 is used on production.
