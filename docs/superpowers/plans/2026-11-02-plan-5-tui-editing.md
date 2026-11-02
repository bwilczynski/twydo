# Twydo Plan 5: TUI editing — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the TUI write: toggle done, due-date shortcuts, delete, undo/redo, move to
another list, and an `i` edit form (title, due, start, repeat interval, notes in `$EDITOR`),
all through the Plan 4 store and gate, with a sync after each write.

**Architecture:** `App` stays free of IO. A write key turns into `Outcome::Write(Request)`;
`term.rs` hands it to a new `Writer` (a thin wrapper over `twydo_core::store::Store`), reloads,
and reports the result back with `App::write_finished`. Due/start text goes through a new pure
parser in `twydo-core` (`dateinput`), which the later natural-language add plan will reuse.
The edit form is a pure state machine in `tdo/src/form.rs`; notes leave the TUI for `$EDITOR`
through `Outcome::EditNotes`.

**Tech Stack:** existing workspace (Rust 2024, ratatui 0.30 / crossterm, chrono, chrono-tz).
Move `tempfile` in `crates/tdo` from dev-dependencies to dependencies (notes temp file).
No other new crates: the date grammar is small and fixed below, so no date-parsing crate.

**Spec:** `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` §7 (creation and editing),
§8 (keys), §10 step 3. Write API: `crates/twydo-core/src/edit/mod.rs` (`Op`, `Change`, `Due`),
`crates/twydo-core/src/store/mod.rs` (`Store::apply/undo_last/redo_last`), `gate.rs`.
Layout: `docs/design/tui-layout.md`. 2Do date shortcuts: `docs/reference/2do/01-features.md` §16.

## Scope

In: `x`/`Space`, `t`, `T`, `]`, `[`, `}`, `{`, `dd`, `u`, `Ctrl-r`, `m` (list picker), `i`,
`Enter` on a leaf, `r` (the form, focused on Repeat), notes in `$EDITOR`, after-edit sync,
and one user session on the test account (Task 8).

Out (user decision 2026-11-02): `o` natural-language add and `O` new item (next plan; it reuses
Task 1's parser); `J`/`K` manual order and list management (Plan 5b). Also out: `a` alerts (no
core support), adding or removing a repeat (no `Change` for it; repeating creates are unverified),
the optional default due time (spec §7; disabled by default), confirmation before completing a
parent with open children (the gate blocks all parent writes today, so it cannot trigger), and
issue #3. Keys that stay out keep the existing `"<action>: not available yet"` message.

## Global Constraints

- **Every write goes through `Store::apply`, `undo_last` or `redo_last`.** The TUI never writes
  a vdir file itself and never calls `gate::check` to decide what to write; it shows the
  store's error text (`WriteError`'s `Display`) as the message.
- **The gate's reason is the user's message.** A blocked write shows exactly `Blocked`'s text
  (e.g. `projects and checklists cannot be edited yet`), never a generic failure.
- **After every write attempt** (success or failure) `term.rs` reloads the snapshot under the
  reader lock and refreshes the sync summary, so a `Stale` failure is followed by fresh data.
- **After every successful write**, when `config.sync` has `after_edit = true`, call
  `crate::cli::sync::spawn_background(cfg, Some(&cfg.vdir))` (Plan 3 Task 10 rule).
- `App` does no IO and reads no clock: dates come from `App::now`; tests drive it with keys.
- UI chrome uses only the 16 ANSI colours via `ui/theme.rs`; true colour only on list bars.
- Tests use `America/New_York`; the reference now is Wed 2026-11-04 10:00 unless stated.
- No personal data in the repo. Test vdirs are copies of committed synthetic fixtures.
- Ask the user before touching production 2Do data, the NAS or the iPhone (Task 8 is the test
  account only). Commit messages carry no agent attribution.

## Review Focus

1. **The vdir changed between load and write** (a sync landed): the write is refused with
   `… changed on disk; reload and try again`, nothing is overwritten, and the view shows the
   new data. Test: Task 2 `stale_snapshot_is_refused_and_untouched`.
2. **The row under the cursor leaves the view** (`x` in Today, `dd`, `m` out of a list view):
   the cursor lands on the row that took its place (or the new last row), never past the end.
   Test: Task 3 `cursor_stays_in_place_after_row_leaves`.
3. **Write keys on an empty view** (or with the sidebar focused): nothing is written, no panic,
   no message. Test: Task 3 `write_keys_on_empty_view_do_nothing`.
4. **The store cannot open** (a stuck or unreadable journal): the TUI still reads; every write
   key shows the reason. Test: Task 2 `writer_without_store_reports_open_error`.
5. **The editor fails or is missing:** notes stay as they were and the message says why; the
   terminal comes back usable. Test: Task 7 `failing_editor_keeps_notes`.

---

### Task 1: Date input parser

**Files:**
- Create: `crates/twydo-core/src/dateinput.rs` (add `pub mod dateinput;` to `lib.rs`)
- Test: `crates/twydo-core/tests/dateinput.rs`

**Interfaces:**
- Consumes: `edit::Due`, `ical::value::IcalDateTime`.
- Produces:
  ```rust
  pub struct DateInput { pub date: NaiveDate, pub time: Option<NaiveTime> }
  impl DateInput { pub fn due(&self) -> Due }          // Time(date.and_time(t)) or Date(date)
  /// Ok(None) for empty/whitespace input (= clear the field).
  pub fn parse(input: &str, today: NaiveDate) -> Result<Option<DateInput>, String>;
  /// "Fri 6 Nov", "Fri 6 Nov 09:00"; year appended when not today's year: "Fri 6 Nov 2027".
  pub fn describe(d: &DateInput, today: NaiveDate) -> String;
  /// The form's text for an existing value: "2026-11-06" or "2026-11-06 09:00"
  /// (Utc/Zoned shown in `tz` local time). `parse` reads it back to the same value.
  pub fn input_text(v: &IcalDateTime, tz: Tz) -> String;
  pub enum DueShift { Today, Tomorrow, Days(i64) }
  pub fn shift_due(current: Option<&IcalDateTime>, tz: Tz, today: NaiveDate, s: DueShift) -> Due;
  ```

**Grammar** (case-insensitive, surrounding whitespace ignored). Input is `[date] [time]`, at
least one present; a time alone means today. Error text: `can't read date "<input>"`.
- date: `today`/`tod`; `tomorrow`/`tom`; `YYYY-MM-DD`; `<n><unit>` with unit
  `d|day|days|w|week|weeks|m|month|months|y|year|years` (today + n; `m` is months as in 2Do;
  months/years with `checked_add_months`, clamping to month end); `<n>st|nd|rd|th` (1–31: the
  first date after today whose day is n, skipping months without it); weekday names
  `sun|sunday|mon|monday|tue|tues|tuesday|wed|wednesday|thu|thur|thurs|thursday|fri|friday|sat|saturday`
  (first such day after today, never today).
- time (the remaining tokens joined without spaces): `midnight` (00:00), `noon` (12:00),
  `H:MM` 24-hour, or `H[[:]MM](a|am|p|pm)` 12-hour (`12a` = 00:00, `12p` = 12:00). A bare
  number such as `9` is an error (ambiguous).

`shift_due`: `Today`/`Tomorrow` set the date to today/today+1; `Days(n)` adds n to the current
due's local date, or to today when there is none. A timed due keeps its local wall time and
becomes `Due::Time` (floating, as 2Do writes); a date-only or missing due stays date-only.

- [ ] **Step 1: Write the failing tests** (today = 2026-11-04, a Wednesday):
  ```rust
  fn p(s: &str) -> Option<DateInput> { parse(s, d(2026,11,4)).unwrap() }
  assert_eq!(p(""), None);  assert_eq!(p("  "), None);
  assert_eq!(p("today"), Some(di(d(2026,11,4), None)));
  assert_eq!(p("TOM"), Some(di(d(2026,11,5), None)));
  assert_eq!(p("3d"), Some(di(d(2026,11,7), None)));
  assert_eq!(p("2w"), Some(di(d(2026,11,18), None)));
  assert_eq!(p("1m").unwrap().date, d(2026,12,4));
  assert_eq!(parse("1m", d(2027,1,31)).unwrap().unwrap().date, d(2027,2,28)); // clamps
  assert_eq!(p("1y").unwrap().date, d(2027,11,4));
  assert_eq!(p("15th").unwrap().date, d(2026,11,15));
  assert_eq!(p("4th").unwrap().date, d(2026,12,4));          // after today, not today
  assert_eq!(parse("31st", d(2026,11,4)).unwrap().unwrap().date, d(2026,12,31)); // Nov has no 31st
  assert_eq!(p("wed").unwrap().date, d(2026,11,11));         // never today
  assert_eq!(p("fri 5pm"), Some(di(d(2026,11,6), Some(t(17,0)))));
  assert_eq!(p("friday 9:30 pm").unwrap().time, Some(t(21,30)));
  assert_eq!(p("3d 930a").unwrap().time, Some(t(9,30)));
  assert_eq!(p("12a").unwrap(), di(d(2026,11,4), Some(t(0,0))));
  assert_eq!(p("noon").unwrap().time, Some(t(12,0)));
  assert_eq!(p("2026-11-20 14:30"), Some(di(d(2026,11,20), Some(t(14,30)))));
  for bad in ["9", "someday", "32nd", "2026-02-30", "fri 25:00", "0d"] {
      assert_eq!(parse(bad, d(2026,11,4)), Err(format!("can't read date \"{bad}\"")));
  }
  ```
  Plus: `describe_formats` (`Fri 6 Nov`, `Fri 6 Nov 09:00`, `Thu 4 Nov 2027`);
  `input_text_round_trips` (Date, Floating, and a `Zoned { tzid: "Europe/Warsaw" }` value shown
  in New York time; `parse(input_text(v))` gives the same local date/time);
  `shift_due_cases` (none + `Days(1)` → `Date(2026-11-05)`; timed 2026-11-01 09:00 + `Days(-1)`
  → `Time(2026-10-31 09:00)`; date-only + `Today` → `Date(today)`; timed + `Tomorrow` keeps
  09:00; timed 2026-10-28 09:00 + `Days(7)` across the DST change on 2026-11-01 →
  `Time(2026-11-04 09:00)`; a `Zoned` due shifts by its New York local date and time).
- [ ] **Step 2:** Run `cargo test -p twydo-core --test dateinput`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement `dateinput.rs` per the grammar above.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): parse due and start date input`.

### Task 2: Writer

**Files:**
- Create: `crates/tdo/src/write.rs` (add `pub mod write;` to `lib.rs`)
- Test: `crates/tdo/tests/write.rs`

**Interfaces:**
- Consumes: `Store::open_with`, `Store::apply`, `Store::undo_last`, `Store::redo_last`,
  `spawn_background`.
- Produces:
  ```rust
  pub enum Request { Apply(Op), Undo, Redo }
  pub struct Writer { /* Result<Store, String>, Config */ }
  impl Writer {
      /// Never fails: an open error is kept and returned by every `run`.
      pub fn open(cfg: &Config) -> Writer;                      // embedded matrix
      pub fn open_with(cfg: &Config, matrix: Matrix) -> Writer;
      /// Ok: the success message. Err: the store's error text.
      pub fn run(&mut self, snap: &Snapshot, req: &Request, now: DateTime<Utc>) -> Result<String, String>;
  }
  ```
  Success messages: Complete `done`; Reopen `reopened`; Delete `deleted · u to undo`;
  Edit `saved`; Move `moved to <list display name or id>`; Undo `undone`; Redo `redone`.
  Open error text: `cannot write: <error>`.

- [ ] **Step 1: Write the failing tests.** Build a temp vdir as `tests/cli_op.rs` `Env::new`
  does (copy `fixtures/phase0/v1/plan4-list1/02-edit/*.ics` into `list1`, empty `list2`,
  `writes.mode = verified`, `state_dir` in the tempdir). Tests:
  - `complete_then_undo_then_redo`: complete `TASK_E` → `Ok("done")`, file has `STATUS:COMPLETED`;
    `Undo` → `Ok("undone")`, file bytes equal the original; `Redo` → `Ok("redone")`.
  - `undo_with_empty_session` → `Err("nothing to undo")`.
  - `blocked_op_returns_gate_reason`: a matrix with no rows → Err equals
    `gate::check(..)`'s `Blocked` text for the same op; no file changed.
  - `stale_snapshot_is_refused_and_untouched` (Review Focus 1): load a snapshot, append a
    line to the task's file, then `run(Edit Summary)` → Err contains `changed on disk`; the
    file holds the appended bytes.
  - `writer_without_store_reports_open_error` (Review Focus 4): a state dir whose identity file
    names another vdir → every `run` returns `Err` starting `cannot write: `.
  - `move_message_names_list`: `Move { to: "list2" }` with a `displayname` file `Two` → `Ok("moved to Two")`.
  - `after_edit_spawns_sync`: `[sync]` with `after_edit = true` and `command` = a script that
    touches a marker → after a successful run the marker appears within 5 s; after an `Err`
    run it does not (reuse the fake-command setup in `tests/cli_op.rs`).
- [ ] **Step 2:** Run `cargo test -p tdo --test write`. Expected: FAIL.
- [ ] **Step 3:** Implement `write.rs`.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tdo): a writer for TUI operations`.

### Task 3: One-key writes in the TUI

**Files:**
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/term.rs`, `crates/tdo/src/ui/mod.rs` (help)
- Test: `crates/tdo/tests/app_keys.rs`

**Interfaces:**
- Consumes: `Request`, `Writer` (Task 2); `dateinput::{shift_due, DueShift}` (Task 1).
- Produces:
  ```rust
  pub enum Outcome { …existing…, Write(Request) }
  impl App { pub fn write_finished(&mut self, result: Result<String, String>); }
  ```
  `write_finished` sets `message` to the Ok or Err text (Tasks 4 and 6 extend it).

Keys (task pane focused, a row selected; otherwise they do nothing — Review Focus 3):
`x`/`Space` → `Complete`, or `Reopen` when the task's status is `Completed`; `t` / `T` /
`]` / `[` / `}` / `{` → `Edit { changes: [Change::Due(Some(shift_due(..)))] }` with
`Today`, `Tomorrow`, `Days(1)`, `Days(-1)`, `Days(7)`, `Days(-7)`; `dd` → `Delete`;
`u` → `Undo`; `Ctrl-r` → `Redo` (`u`/`Ctrl-r` work from any pane, also on an empty view).

`term.rs`: open `Writer::open(&cfg)` once in `run` (after the existing startup recovery). On
`Outcome::Write(req)`: `let r = writer.run(&app.snapshot, &req, Utc::now());` then
`reload(app, root)`, `refresh_sync(app)`, `app.write_finished(r)`. Help overlay: drop the
`(after gate)` mark from the keys this plan enables.

- [ ] **Step 1: Write the failing tests** (in-memory app, as the existing tests do):
  - `x_requests_complete_or_reopen`: on an open task `x` → `Outcome::Write(Request::Apply(Op::Complete { uid }))`;
    in Done view on a completed task `Space` → `Reopen`.
  - `date_keys_request_due_edits`: for each of `t T ] [ } {` the outcome's change equals
    `shift_due(task.due, NY, 2026-11-04, …)` with the matching shift.
  - `dd_requests_delete_and_single_d_does_not`.
  - `u_and_ctrl_r_request_undo_redo`, also from the sidebar and on an empty view.
  - `write_keys_on_empty_view_do_nothing` (Review Focus 3): an empty view and sidebar focus:
    `x t ] dd` → `Outcome::Continue`, `message == None`.
  - `cursor_stays_in_place_after_row_leaves` (Review Focus 2): cursor on row 1 of 3 in Today;
    `app.reload(snapshot_without(that task))` → cursor 1, on the former row 2; cursor on the
    last row → the new last row.
  - `write_finished_shows_message`: `Err("projects and checklists cannot be edited yet")` →
    `message` equals it.
- [ ] **Step 2:** Run `cargo test -p tdo --test app_keys`. Expected: FAIL.
- [ ] **Step 3:** Implement the keys, `write_finished`, the `term.rs` wiring and the help change.
  If `cursor_stays_in_place_after_row_leaves` already passes (via `restore`), keep it as a guard.
- [ ] **Step 4:** Run `cargo test -p tdo`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): toggle done, due shortcuts, delete, undo and redo`.

### Task 4: Move to another list (`m`)

**Files:**
- Create: `crates/tdo/src/ui/picker.rs`
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/ui/mod.rs`
- Test: `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct ListPicker { pub uid: String, pub lists: Vec<String>, pub cursor: usize }
  pub picker: Option<ListPicker>   // field on App
  ```
  `lists` = the collection ids of the `View::List` entries of `sidebar_items()`, in that order,
  without the task's own collection. No other list → message `no other list to move to`.
  Picker keys: `j`/`k` move, `Enter` → `Outcome::Write(Apply(Op::Move { uid, to }))` (picker
  stays open until `write_finished`, which closes it on Ok and on Err), `Esc` closes.
  Rendered as a centred overlay titled ` Move to `, each line `▌ <name>` with the list colour
  bar, the cursor row in the selection style used by the task list.
- [ ] **Step 1: Write the failing tests:** `m_opens_picker_without_current_list`;
  `picker_enter_requests_move`; `picker_esc_closes`; `m_with_single_list_shows_message`;
  render test `picker_overlay_lists_names` (an 80×24 buffer contains ` Move to ` and the names).
- [ ] **Step 2:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p tdo`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): move a task to another list`.

### Task 5: Edit form state

**Files:**
- Create: `crates/tdo/src/form.rs` (add `pub mod form;` to `lib.rs`)
- Test: `crates/tdo/tests/form.rs`

**Interfaces:**
- Consumes: `dateinput::{parse, describe, input_text}`, `model::repeat::parse_rrule`,
  `edit::Change`.
- Produces:
  ```rust
  pub enum Field { Title, Due, Start, Repeat, Notes }
  pub enum FormAction { None, Cancel, Save, EditNotes }
  pub struct EditForm {
      pub uid: String, pub focus: Field,
      pub title: String, pub due: String, pub start: String,
      /// None: the task has no parseable RRULE (row shows "none", not focusable).
      pub repeat: Option<String>, pub repeat_unit: &'static str,   // "days" | "weeks" | "months" | "years"
      pub notes: String, pub error: Option<String>,
      /* initial values */
  }
  impl EditForm {
      pub fn open(task: &Task, tz: Tz) -> EditForm;   // due/start via input_text; repeat = interval
      pub fn fields(&self) -> Vec<Field>;             // Title, Due, Start, [Repeat], Notes
      pub fn key(&mut self, k: KeyEvent) -> FormAction;
      /// "→ Fri 6 Nov 09:00", "→ none" (empty) or the parse error; Due and Start only.
      pub fn preview(&self, f: Field, today: NaiveDate) -> Option<String>;
      /// Only fields whose text differs from the initial text, in field order.
      pub fn changes(&self, today: NaiveDate) -> Result<Vec<Change>, String>;
      pub fn set_notes(&mut self, notes: String);
  }
  ```
Keys: printable chars append to the focused text field (not Notes); `Backspace` removes one
`char`; `Ctrl-u` clears the field; `Ctrl-w` removes the last word and the spaces before it;
`Tab`/`Down` next field, `BackTab`/`Up` previous (wrapping, skipping an absent Repeat); `Enter`
on Notes → `EditNotes`, elsewhere → `Save`; `Ctrl-s` → `Save`; `Esc` → `Cancel`. Repeat accepts
digits only.

`changes` errors: empty title → `title must not be empty`; bad date → the parser's text
prefixed `due: ` / `start: `; a time in Start → `start: start dates have no time`; repeat not
a number ≥ 1 → `repeat: enter a number of <unit>`. Mapping: Title → `Summary`; Due →
`Due(parse(..).map(|d| d.due()))`; Start → `Start(parse(..).map(|d| d.date))`; Repeat →
`RepeatInterval(n)`; Notes → `Description(None)` when empty, else `Some(notes)`.

- [ ] **Step 1: Write the failing tests:** `open_fills_fields` (a timed task shows
  `2026-11-06 09:00`; a non-repeating task has `repeat == None` and `fields()` lacks Repeat);
  `unchanged_form_has_no_changes`; `typed_due_becomes_change` (`Ctrl-u`, `fri 5pm` →
  `[Due(Some(Time(2026-11-06 17:00)))]`); `cleared_due_is_none`; `start_with_time_is_error`;
  `empty_title_is_error`; `repeat_interval_change`; `backspace_and_ctrl_w_are_char_aware`
  (title `Zażółć gęślą` + Backspace → `Zażółć gęśl`; then `Ctrl-w` → `Zażółć`; `Ctrl-w` → `""`); `tab_cycles_and_skips_absent_repeat`; `enter_on_notes_asks_for_editor`;
  `preview_shows_parsed_or_error`.
- [ ] **Step 2:** Run `cargo test -p tdo --test form`. Expected: FAIL.
- [ ] **Step 3:** Implement `form.rs`.
- [ ] **Step 4:** Run it again. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): edit form state`.

### Task 6: Edit form in the TUI (`i`, `Enter`, `r`)

**Files:**
- Create: `crates/tdo/src/ui/form.rs`
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/ui/{mod,status}.rs`
- Test: `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/ui_render.rs`

**Interfaces:**
- Consumes: `EditForm`, `FormAction` (Task 5).
- Produces: `Mode::Insert`; `pub form: Option<EditForm>` on `App`. `write_finished(Ok)` closes
  the form; `write_finished(Err(e))` keeps it open with `form.error = Some(e)`.

`i` (any task) and `Enter` on a leaf open the form with focus Title; `r` opens it with focus
Repeat, or, when the task has none, shows `edit repeat: this task does not repeat`. While the
form is open, `mode == Insert` and keys go to `EditForm::key`: `Cancel` closes; `Save` →
`changes(today)`: `Err` → `form.error`; `Ok(empty)` → close with message `no changes`;
`Ok(changes)` → `Outcome::Write(Apply(Op::Edit { uid, changes }))`. (`EditNotes` is Task 7;
until then it does nothing.)

Rendering: a centred overlay ` Edit task `, one row per field (`Title`, `Due`, `Start`,
`Repeat` as `every <n> <unit>` or `none`, `Notes` as its first line plus `(+N lines)`), the
focused field in the accent style with a `▏` cursor, previews under Due and Start in dim
style, `form.error` in red at the bottom. Status line: `INSERT` badge and hints
`Tab next field  Enter save  Esc cancel` (Notes focused: `Enter edit in $EDITOR`).

- [ ] **Step 1: Write the failing tests:** `i_opens_form_in_insert_mode`;
  `enter_on_leaf_opens_form_and_on_parent_toggles` (existing parent behaviour kept);
  `r_focuses_repeat_or_explains`; `save_requests_edit_with_changes`;
  `save_without_changes_closes_with_message`; `failed_write_keeps_form_open_with_error`;
  `esc_cancels_without_write`; render tests `form_overlay_shows_fields_and_preview` and
  `status_shows_insert_mode`.
- [ ] **Step 2:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p tdo`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): edit a task in a form`.

### Task 7: Notes in `$EDITOR`

**Files:**
- Create: `crates/tdo/src/editor.rs` (add `pub mod editor;` to `lib.rs`)
- Modify: `crates/tdo/src/app.rs`, `crates/tdo/src/term.rs`, `crates/tdo/Cargo.toml`
- Test: `crates/tdo/tests/editor.rs`, `crates/tdo/tests/app_keys.rs`

**Interfaces:**
- Produces:
  ```rust
  /// $VISUAL, else $EDITOR, else "vi" (first non-empty).
  pub fn editor_command(visual: Option<&str>, editor: Option<&str>) -> String;
  /// Writes `text` to a temp `notes.md`, runs `sh -c '<editor> "$1"' sh <path>` with the
  /// terminal's stdio, reads the file back with one trailing "\n" removed.
  /// Err(message) when the editor cannot start or exits non-zero.
  pub fn edit_text(editor: &str, text: &str) -> Result<String, String>;
  pub enum Outcome { …, EditNotes(String) }
  impl App { pub fn notes_edited(&mut self, result: Result<String, String>); }
  ```
  `FormAction::EditNotes` → `Outcome::EditNotes(form.notes.clone())`. `term.rs`:
  `ratatui::restore()`, `edit_text(..)`, `*terminal = ratatui::init()`, `app.resize(..)`,
  `app.notes_edited(r)`. `notes_edited(Ok)` → `form.set_notes`; `Err(e)` → `form.error =
  Some(format!("notes: {e}"))`, notes unchanged.
- [ ] **Step 1: Write the failing tests:** `editor_command_precedence`;
  `edit_text_returns_new_content` (editor `sed -i s/old/new/` on `old\n` → `new`);
  `edit_text_strips_one_newline` (editor writing `a\n\n` → `a\n`);
  `failing_editor_keeps_notes` (Review Focus 5: editor `false` → `Err`; then
  `app.notes_edited(Err(..))` leaves `form.notes` as before and sets `form.error`);
  `enter_on_notes_requests_editor` (app outcome carries the current notes).
- [ ] **Step 2:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 3:** `cargo add -p tdo tempfile` (and remove it from dev-dependencies); implement.
- [ ] **Step 4:** Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`.
  Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(tui): edit notes in $EDITOR`.

### Task 8: Verify on the test account, then document

**Files:**
- Create: `docs/tui.md` (keys, what each write does, what stays blocked and why)
- Modify: `docs/reference/phase0/README.md` or a new `docs/reference/phase0/plan5-tui.md` (session results),
  `AGENTS.md` ("What to do next")

Ask the user first; use the test setup from the memory notes (`XDG_CONFIG_HOME=~/twydo-phase0/xdg`
only for `tdo`, test lists "Twydo test" / "Twydo test 2", synthetic single-letter titles,
rounds of ≤6 phone actions).

- [ ] **Step 1:** `cargo build --release -p tdo` in the worktree. With the user, in the TUI
  on the test lists: `x` and `u` on a one-time task; `]` then `t` on a monthly task;
  `m` a one-time task to "Twydo test 2"; `i` with a new title, `fri 5pm` due and a two-line
  note from `$EDITOR`; `dd` then `u`. `:sync`, then the user checks each on the iPhone and
  edits the multi-line-note task there; sync back and confirm the TUI shows the phone's edit.
- [ ] **Step 2:** Record the results (what was checked, pass/fail, anything new) in
  `docs/reference/phase0/plan5-tui.md`. A failure becomes a fix task here before merging; a new 2Do
  behaviour goes to `compat/matrix.toml` only with captured evidence.
- [ ] **Step 3:** Write `docs/tui.md`; update AGENTS.md "What to do next" to: next is the
  natural-language add plan, Plan 5b (manual order, list management) or Plan 2 (notifier).
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `docs: TUI editing keys and test-account verification`.
