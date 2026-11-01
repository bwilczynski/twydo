# Twydo Plan 4: Writable core — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let Twydo change task resources safely: preserving field edits, one-time completion/reopen,
2Do-style copy-and-advance for the verified monthly/yearly rules, create, move, delete, and
undo/redo. Every write goes through a durable journal and a compatibility gate. A scriptable
`tdo op` CLI drives the operations, so they can be verified against the iPhone before the TUI uses them (Plan 5).

**Architecture:** `twydo-core` gains four layers. (1) An iCalendar write layer that re-emits only
edited lines. (2) Pure resource edits on a parsed `Calendar`, written to match what 2Do itself
writes, as captured in fixtures. (3) A `Store` that applies an `Op` atomically under a lock, through
a JSON operation journal outside the vdir, with recovery and undo/redo. (4) A gate that allows an
`Op` only when the compatibility matrix (embedded in the binary) and the verified-shape rules allow it.
`tdo` adds `tdo op …` and runs recovery before any command reads the vdir.

**Tech Stack:** existing workspace (Rust 2024, chrono, serde/serde_json/toml, thiserror, clap).
New: `uuid` (feature `v4`) in `twydo-core`; enable `serde_json`'s `preserve_order` feature.
File locking uses `std::fs::File::lock` (stable in the pinned toolchain). Add crates with `cargo add`.

**Spec:** `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` §5 (repeat rules, parent
tasks), §6 (preservation and editing, recurring completion, recovery), §7 (creation and editing),
§9 (round trips, failures). 2Do encodings: `docs/reference/2do/06-sync.md` §2.6.

## Scope

In: field edits (title, notes, due, start, repeat interval), one-time complete/reopen, recurring
complete for the verified shape only, create (plain task), move, delete, undo/redo, journal and
recovery, gate, `tdo op`, and two user sessions on the test account (Task 2 baseline captures,
Task 12 Twydo→iPhone verification).

Out (later plans, or blocked with a reason until a capture verifies them): alarm editing; project,
checklist and child operations; days/weeks/weekday-set/from-completion/count/until rules; late
recurring completion; recurrence overrides; TUI keys (Plan 5); conflict store, sync confirmation
and journal pruning (Plan 3); list management (Plan 5).

## Global Constraints

- **Preservation (spec §6, §9):** a resource no operation targets is never rewritten. An edited resource
  differs from its input only in the target properties, `DTSTAMP`, `LAST-MODIFIED` and the verified
  metadata counterparts; every other line keeps its original bytes (folds and line endings included).
  `SEQUENCE` is left alone (2Do does not bump it; fixtures `rule-*/02-complete`, `03-edit`).
- **Every write is journaled, atomic and under the vdir lock.** Readers in every entry point
  (`tui`, `export-json`, `check`, `op`) run recovery first (spec §6 Recovery).
- **Gate:** no operation runs unless `gate::check` allows it. The default write mode is `verified`.
  `test` mode is only for the isolated test account; every `tdo op` run in `test` mode prints
  `tdo: test write mode: unverified operations are enabled` to stderr.
- **No library defaults for unresolved 2Do behaviour.** If a case has not been captured, block it
  with a reason. Do not guess.
- **Hypotheses H1–H12 (Task 2) drive Tasks 5–7.** If a capture contradicts one, stop and amend this
  plan before implementing the task that depends on it.
- Ask the user before any action that touches production 2Do data, the NAS or the iPhone. Task 2
  and Task 12 use the **test account only**, with synthetic titles.
- No personal data in the repo; fixtures are committed only after `tools/capture_fixture.py scan`
  passes and the user confirms the content is synthetic.
- Tests use `America/New_York`. Writes use an injected `now: DateTime<Utc>`; tests never read the clock.
- Commit messages have no agent attribution (user's global instructions).

## Review Focus

1. **The file changed on disk between load and write** (vdirsyncer pulled an iPhone edit): refuse
   with `WriteError::Stale` and write nothing. Test → Task 10 (`apply_refuses_when_file_changed_since_load`).
2. **A crash halfway through a completion** (history copy written, live task not yet advanced):
   recovery finishes it exactly once, with no second copy and no lost advance. Tests → Task 8
   (`recover_rolls_forward_half_applied`, `recover_is_idempotent`).
3. **Undo after the resource changed elsewhere:** refuse; never overwrite the newer version.
   Test → Task 10 (`undo_refuses_after_external_change`).
4. **Titles/notes with commas, semicolons, newlines, backslashes and long non-ASCII text:** escaped
   and folded correctly, and they read back equal. Test → Task 3 (`edited_text_escapes_folds_and_reads_back`).
5. **Completing a monthly task so late that one advance still lands in the past:** blocked with
   the edge-case reason, never silently skipped or clamped. Test → Task 9 (`late_recurring_completion_blocked`).

---

## File structure

```
crates/twydo-core/src/
  compat.rs            + iphone_baseline column, Matrix::embedded(), combined_verdicts (moved from tdo)
  config.rs            + [writes] mode, state_dir
  vdir.rs              + LoadReport.sources (raw bytes per file)
  views.rs             + Snapshot::source(), Snapshot::find()
  ical/mod.rs          + write API: dirty properties, insert/remove, folding, line-ending detection
  ical/value.rs        + escape_text, format_date_time
  model/meta_edit.rs   new: <2Do Meta> JSON editor that keeps bytes outside edited fields
  edit/mod.rs          new: Op, Change, Due, NewTask, EditError, field edits
  edit/complete.rs     new: one-time complete/reopen, recurring copy-and-advance, next_due
  edit/create.rs       new: new_task(), new_uid()
  gate.rs              new: WriteMode-aware policy, Blocked reasons
  store/mod.rs         new: Store (apply, undo, redo, recover)
  store/fsio.rs        new: atomic write/remove, VdirLock
  store/journal.rs     new: journal entries, recovery
crates/tdo/src/cli/op.rs   new: `tdo op …`
docs/journal.md            new: journal format
docs/phase0/plan4-baseline.md  new: capture checklist + findings (Tasks 2, 12)
compat/matrix.toml         + iphone_baseline on every [[operation]]
```

---

### Task 1: Matrix `iphone_baseline` column, embedded matrix, write config

**Files:**
- Modify: `crates/twydo-core/src/compat.rs`, `compat/matrix.toml`, `crates/twydo-core/src/config.rs`, `crates/tdo/src/cli/check.rs`, `docs/config.md`
- Test: `crates/twydo-core/src/compat.rs` (unit), `crates/twydo-core/tests/config.rs`

**Interfaces:**
- Produces:
  ```rust
  // compat.rs
  pub struct OperationEntry { …existing…, pub iphone_baseline: Verdict }   // required field
  impl Matrix {
      pub fn embedded() -> &'static Matrix;          // include_str!("../../../compat/matrix.toml"), parsed once (OnceLock)
      pub fn operation(&self, case: &str, op: &str) -> Option<&OperationEntry>;
      pub fn edge_case(&self, id: &str) -> Option<&EdgeCase>;
  }
  pub fn combined_verdicts(m: &Matrix, class: &str) -> (Verdict, Verdict, Verdict); // moved from tdo check.rs, same semantics
  // config.rs
  #[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)] #[serde(rename_all = "lowercase")]
  pub enum WriteMode { #[default] Verified, Test }
  pub struct Writes { pub mode: WriteMode }                 // [writes] table, deny_unknown_fields
  pub struct Config { …, pub writes: Writes, pub state_dir: Option<PathBuf> }
  impl Config { pub fn state_dir(&self) -> PathBuf; }       // state_dir, else $XDG_STATE_HOME/twydo, else ~/.local/state/twydo
  pub fn state_dir_from(xdg_state: Option<&Path>, home: &Path) -> PathBuf;   // empty xdg = unset
  ```
  `iphone_baseline` means: the iPhone's own resource changes for this case/op are captured and understood, so Twydo knows what to write.

- [ ] **Step 1: Write the failing tests:**
  - in `compat.rs`: `embedded_matrix_has_baseline_column`: `Matrix::embedded().operation("one-time","create").unwrap().iphone_baseline == Verdict::Unverified`; 66 operations; `matrix_template_loads` still passes.
  - `combined_verdicts_takes_worst_over_modifiers` (moved from tdo): `months-day-1-28` → all `Pass`; `months-day-1-28+count` → execute `Unverified`.
  - in `tests/config.rs`: `writes_mode_defaults_to_verified`; `writes_mode_test_parses` (`[writes]\nmode = "test"`); `writes_mode_rejects_unknown` (`mode = "yolo"` is a parse error); `state_dir_from_xdg_and_home` (`Some("/x")` → `/x/twydo`; `None` / `Some("")` with home `/h` → `/h/.local/state/twydo`).
- [ ] **Step 2:** Run `cargo test -p twydo-core`. Expected: FAIL (missing field/functions).
- [ ] **Step 3:** Add `iphone_baseline = "unverified"` to all 66 `[[operation]]` rows (place it before `mac_to_iphone`) and document it in the matrix header comment. Implement the interfaces above. Point `tdo check` at the core `combined_verdicts`. Document `[writes] mode` and `state_dir` in `docs/config.md`. In particular, say that `test` must only point at the test account.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): embedded matrix with iphone_baseline column; write mode config`.

### Task 2: iPhone baseline captures (👤 user + 🤖, test account only)

**Files:**
- Create: `docs/phase0/plan4-baseline.md`, `fixtures/phase0/v1/<scenario>/<step>/…` (after scan)
- Modify: `compat/matrix.toml`

This task produces the evidence that Tasks 5–7 encode. **Stop and confirm with the user** before
starting. They need the test 2Do on the iPhone, the test CalDAV account, and vdirsyncer set up
as in `docs/phase0/README.md` steps 3–4. Use synthetic titles only.

- [ ] **Step 1:** Write `docs/phase0/plan4-baseline.md`: the steps below as a 👤/🤖 checklist, the capture command (from `docs/phase0/README.md` step 6, `--device iphone`), and the hypotheses table. A move is captured as `NN-move-source` (source folder) and `NN+1-move-dest` (destination folder). Every action happens on the iPhone; after each one: sync on the phone, run `vdirsyncer sync twydo`, then capture.

  | Scenario | Steps |
  |---|---|
  | `one-time` | 01-create `Test one-time` due 2026-12-03, date only · 02-edit-title · 03-edit-notes (text with a comma and a newline) · 04-clear-notes · 05-complete · 06-reopen · 07-move-source / 08-move-dest · 09-delete |
  | `date-only` | 01-create due 2026-12-03 · 02-edit-date to 2026-12-05 · 03-remove-date |
  | `timed` | 01-create due 2026-12-03 17:30 (record the phone's zone in `--action`) · 02-edit-time to 18:45 · 03-make-date-only |
  | `start-date` | 01-create start 2026-12-01, due 2026-12-03 · 02-edit-start to 2026-12-02 · 03-remove-start |
  | `monthly` | 01-create monthly due 2026-12-10 · 02-edit-notes · 03-complete · 04-reopen-history-copy · 05-move-source / 06-move-dest · 07-delete |
  | `yearly` | 01-create yearly due 2026-12-15 · 02-move-source / 03-move-dest · 04-delete |

  Hypotheses (record ✅/❌ and the evidence step for each; any ❌ → stop, amend Tasks 5–7 with the user):

  | # | Hypothesis |
  |---|---|
  | H1 | Edit title/notes changes only `SUMMARY`/`DESCRIPTION`, `DTSTAMP`, `LAST-MODIFIED` |
  | H2 | Cleared notes are written as an empty `DESCRIPTION:` |
  | H3 | Complete (one-time) sets `STATUS:COMPLETED`, adds `PERCENT-COMPLETE:100` and `COMPLETED:<UTC>`; nothing else beyond timestamps |
  | H4 | Reopen (one-time) sets `STATUS:NEEDS-ACTION` and removes `COMPLETED` and `PERCENT-COMPLETE` |
  | H5 | Move = same `UID` and content in the destination folder (only timestamps may change); source file removed |
  | H6 | Delete removes the file; no other resource changes |
  | H7 | Due edit changes `DUE` and its `DTSTART` copy together; removing the due date removes both |
  | H8 | A timed due is floating local time `DUE:YYYYMMDDTHHMMSS` (no `TZID`, no `Z`), and `DTSTART` copies it |
  | H9 | Start edit changes only `StartDate` in `<2Do Meta>` (epoch seconds of UTC midnight of the start day); removing it deletes the key |
  | H10 | 2Do creates a plain task with the properties and metadata key order seen in `rule-*/01-create`, with `RecurrenceValue`/`RecurrenceType`/`RecurrenceFrom` = 0 and `RUID` = its own UID |
  | H11 | Completing a monthly task again matches `rule-months-day-1-28/02-complete` (advance live, add copy) |
  | H12 | Moving or deleting a recurring task touches only its live resource, not its history copies |

- [ ] **Step 2 (👤+🤖):** Run the captures into `$P0/staging`. Then run `python3 tools/capture_fixture.py scan "$P0/staging"`. It must exit 0. Then ask the user to confirm the content is synthetic and copy to `fixtures/phase0/v1/`.
- [ ] **Step 3:** Fill in the hypotheses table and observations in `plan4-baseline.md`. Set `iphone_baseline = "pass"` (with `evidence` paths) for each case/op whose capture matches its hypotheses: `one-time` all six; `date-only`, `timed`, `start-date`: create and edit; `monthly`: create, edit, complete, move, delete; `yearly`: create, edit, complete, move, delete (create/edit/complete evidence from `rule-years`). `monthly` reopen: record what was observed; set it to `pass` only if the result is fully understood.
- [ ] **Step 4:** Run `cargo test --workspace` (the embedded matrix must still load). Expected: PASS.
- [ ] **Step 5:** Commit: `test: iPhone baseline fixtures for plan-4 operations`.

### Task 3: iCalendar write layer

**Files:**
- Modify: `crates/twydo-core/src/ical/mod.rs`, `crates/twydo-core/src/ical/value.rs`
- Test: `crates/twydo-core/tests/ical_write.rs`, synthetic data in `crates/twydo-core/tests/data/ical/`

**Interfaces:**
- Consumes: `Calendar`, `Component`, `Property`, `Param`, `parse`, `serialize` (Plan 1).
- Produces:
  ```rust
  impl Property {
      pub fn new(name: &str, params: Vec<Param>, value: &str) -> Property;   // dirty: no raw bytes
      pub fn set_value(&mut self, value: &str);                               // marks dirty, keeps params
      pub fn set_params(&mut self, params: Vec<Param>);                       // marks dirty
  }
  impl Component {
      pub fn prop_mut(&mut self, name: &str) -> Option<&mut Property>;        // first match
      pub fn set_prop(&mut self, p: Property);      // replace first same-name prop in place, else insert_prop
      pub fn insert_prop(&mut self, p: Property, after: Option<&str>); // after the first prop named `after`; else after the last property, before child components
      pub fn remove_props(&mut self, name: &str) -> usize;
      pub fn children_named_mut<'a>(&'a mut self, name: &'a str) -> impl Iterator<Item = &'a mut Component>;
  }
  impl Calendar { pub fn line_ending(&self) -> &'static str; }  // "\r\n" or "\n", taken from the BEGIN:VCALENDAR line
  pub fn new_calendar(root: Component) -> Calendar;             // for created resources (Task 7)
  impl Component { pub fn new(name: &str) -> Component; pub fn push_child(&mut self, c: Component); }
  // value.rs
  pub fn escape_text(s: &str) -> String;                     // \ ; , newline → \\ \; \, \n
  pub fn format_date_time(v: &IcalDateTime) -> (Vec<Param>, String);  // Date → VALUE=DATE + YYYYMMDD; Floating → YYYYMMDDTHHMMSS; Utc → …Z; Zoned → TZID=…
  ```
  `serialize` writes clean properties from `raw` as before. It writes dirty ones as `NAME;PARAM=v:value`, with each parameter value DQUOTEd when it contains `:;,`, folded so each physical line is at most 75 octets (continuations start with one space, never split a UTF-8 sequence), and ended with `cal.line_ending()`.

- [ ] **Step 1: Write the failing tests:**
  - `unedited_calendar_is_byte_equal`: every phase-0 fixture `.ics` → `serialize(parse(b)) == b` after calling `prop_mut("SUMMARY")` without changing anything.
  - `edit_changes_only_that_line`: on `fixtures/phase0/v1/rule-years/01-create/c3e7…ics`, `set_value` on `SUMMARY` → output differs from input in that logical line only (compare logical-line lists).
  - `insert_and_remove_keep_other_bytes`: insert `PERCENT-COMPLETE:100` after `STATUS`, remove `RRULE`; every other logical line is unchanged and in the same order; a `VALARM` child stays after the properties.
  - `new_lines_use_file_line_ending`: a CRLF file gets CRLF on the edited line; an LF file gets LF.
  - `edited_text_escapes_folds_and_reads_back`: set `SUMMARY` to `escape_text("Zażółć, gęślą; jaźń\\ 🙂\nline two ".repeat(4))`. No physical line is over 75 octets. Re-parsing then `unescape_text` gives the original string.
  - `format_date_time_forms`: `Date(2026-12-03)` → `([VALUE=DATE], "20261203")`; `Floating(2026-12-03T17:30)` → `([], "20261203T173000")`; `Utc` → `"…Z"`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test ical_write`. Expected: FAIL.
- [ ] **Step 3:** Implement. Track dirtiness as `raw: Option<Vec<u8>>` (`None` = re-emit). Keep the `layout` slots consistent on insert and remove.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS (including Plan 1 round-trip tests).
- [ ] **Step 5:** Commit: `feat(core): iCalendar write layer that re-emits only edited lines`.

### Task 4: `<2Do Meta>` JSON editing

**Files:**
- Create: `crates/twydo-core/src/model/meta_edit.rs` (export from `model/mod.rs`)
- Modify: `crates/twydo-core/Cargo.toml` (`serde_json` feature `preserve_order`)
- Test: `crates/twydo-core/tests/meta_edit.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct MetaJson { /* prefix before "<2Do Meta>", ordered JSON object, suffix after "</2Do Meta>" (e.g. "\\n") */ }
  impl MetaJson {
      pub fn parse(raw_value: &str) -> Option<MetaJson>;   // raw (still TEXT-escaped) X-2DOAPP-METADATA value; None unless <2Do Meta> JSON object
      pub fn encode(&self) -> String;                      // inverse of parse
      pub fn get_i64(&self, key: &str) -> Option<i64>;
      pub fn set_i64(&mut self, key: &str, v: i64);         // replace in place; append at end if absent
      pub fn set_str(&mut self, key: &str, v: &str);
      pub fn remove(&mut self, key: &str);
  }
  pub fn default_task_meta(uid: &str) -> MetaJson;  // H10 key set and order, zero values, RUID = uid
  ```
  Percent-encode every byte except ASCII letters, digits and `-._~`. The round-trip test below checks this rule against 2Do's own output.

- [ ] **Step 1: Write the failing tests:**
  - `encode_parse_is_identity_on_fixtures`: for every `X-2DOAPP-METADATA` value in `fixtures/phase0/v1/**.ics`, `MetaJson::parse(v).unwrap().encode() == v`.
  - `set_keeps_key_order`: on the `rule-months-day-1-28/01-create` value, `set_i64("RecurrenceValue", 2)` → decoded JSON keys keep their order; `encode` equals the `03-edit` value of that resource.
  - `remove_and_append`: `remove("StartDate")` on a value without it is a no-op; `set_i64("StartDate", 1796256000)` appends `StartDate` as the last key.
  - `default_task_meta_matches_2do`: equals the decoded metadata of the H10 evidence resource with `RUID` replaced (taken from Task 2's one-time `01-create`).
  - `non_json_is_none`: token-format `@tasktype:project` → `None`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test meta_edit`. Expected: FAIL.
- [ ] **Step 3:** Implement with `serde_json::Map` (preserve_order). Keep `prefix`/`suffix` verbatim.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): edit 2Do JSON metadata without reordering it`.

### Task 5: Field edits

**Files:**
- Create: `crates/twydo-core/src/edit/mod.rs` (export `pub mod edit` from `lib.rs`)
- Test: `crates/twydo-core/tests/edit_fields.rs`

**Interfaces:**
- Consumes: Task 3 write API, Task 4 `MetaJson`, `parse_rrule` (Plan 1).
- Produces:
  ```rust
  pub enum Due { Date(NaiveDate), Time(NaiveDateTime) }     // Time = floating local (H8)
  pub enum Change { Summary(String), Description(Option<String>), Due(Option<Due>), Start(Option<NaiveDate>), RepeatInterval(u32) }
  pub struct NewTask { pub collection: String, pub summary: String, pub description: Option<String>, pub due: Option<Due>, pub start: Option<NaiveDate> }
  pub enum Op {
      Create(NewTask), Edit { uid: String, changes: Vec<Change> }, Complete { uid: String },
      Reopen { uid: String }, Move { uid: String, to: String }, Delete { uid: String },
  }
  #[derive(thiserror::Error, Debug)] pub enum EditError {
      #[error("title must not be empty")] EmptyTitle,
      #[error("no VTODO in resource")] NoTodo,
      #[error("{0}")] Unsupported(String),
  }
  pub fn apply_changes(cal: &mut Calendar, changes: &[Change], now: DateTime<Utc>) -> Result<(), EditError>;
  pub fn touch(todo: &mut Component, now: DateTime<Utc>);   // DTSTAMP and LAST-MODIFIED = now (UTC); inserted if absent
  ```
  Rules (from H1, H2, H7–H9, and `rule-*/03-edit`):
  - `Summary`: trimmed and nonempty, else `EmptyTitle`; written with `escape_text`.
  - `Description(None)`: an existing `DESCRIPTION` becomes empty (H2); if it is absent, it stays absent. `Some(s)`: set, inserted after `SUMMARY` if absent.
  - `Due`: set or remove `DUE` (dropping `TZID`/`VALUE` params that no longer fit). When the task has `<2Do Meta>` JSON, `DTSTART` mirrors `DUE` (set, change and remove together). Without JSON metadata, `DTSTART` is a real start and stays untouched. A time of 00:00 → `Unsupported("a 00:00 due time is not supported yet (2Do's duetime encoding is not captured)")`.
  - `Start`: requires `<2Do Meta>` JSON (token metadata → `Unsupported("this task's 2Do metadata format is read-only")`). Without any metadata, insert `default_task_meta(uid)` first. `Some(d)` → `StartDate` = epoch seconds of `d` 00:00 UTC; `None` → remove `StartDate`.
  - `RepeatInterval(n)`: `n >= 1`. `RRULE` gets `INTERVAL=n` appended (or replaced); `n == 1` removes `INTERVAL`. JSON `RecurrenceValue` = n.
  - Every successful call runs `touch` once.

- [ ] **Step 1: Write the failing tests** (input resources come from Task 2 / phase-0 fixtures; `now` = the evidence step's `DTSTAMP`). Each one checks that the changed logical lines are exactly the expected set and that their values equal the evidence values:
  - `summary_edit_matches_2do` (one-time 01→02), `notes_edit_and_clear_match_2do` (03, 04)
  - `due_edit_and_remove_match_2do` (date-only 02, 03), `timed_due_edit_matches_2do` (timed 02, 03)
  - `start_edit_and_remove_match_2do` (start-date 02, 03)
  - `repeat_interval_edit_matches_2do`: `rule-months-day-1-28` 02→03 gives `RRULE:FREQ=MONTHLY;INTERVAL=2` and `RecurrenceValue` 2; the same for `rule-years`.
  - `empty_title_rejected`: `Summary("  ")` → `EmptyTitle`, calendar unchanged.
  - `dtstart_without_json_meta_is_left_alone`: a synthetic task with no metadata and its own `DTSTART` keeps `DTSTART` when `DUE` changes.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test edit_fields`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): preserving field edits with 2Do metadata counterparts`.

### Task 6: Completion, reopen, recurring copy-and-advance

**Files:**
- Create: `crates/twydo-core/src/edit/complete.rs`
- Test: `crates/twydo-core/tests/edit_complete.rs`

**Interfaces:**
- Consumes: Task 5 (`touch`, `EditError`), `RRule`/`parse_rrule`, `MetaJson`.
- Produces:
  ```rust
  pub fn complete_one_time(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError>;  // H3; PERCENT-COMPLETE and COMPLETED inserted after STATUS
  pub fn reopen_one_time(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError>;    // H4
  /// Verified shape only: date-only DUE, RRULE = FREQ=MONTHLY|YEARLY with optional INTERVAL and nothing else.
  pub fn next_due(due: NaiveDate, rule: &RRule) -> Result<NaiveDate, EditError>;
  /// Returns (advanced live resource, completed history copy).
  pub fn complete_recurring(cal: &Calendar, now: DateTime<Utc>, copy_uid: &str) -> Result<(Calendar, Calendar), EditError>;
  ```
  Copy-and-advance as observed in `rule-months-day-1-28/02-complete` and `rule-years/02-complete`:
  - Live: `DUE` and its `DTSTART` copy → `next_due`; `touch`. Nothing else changes.
  - Copy (cloned from the input before the advance): `UID` = `copy_uid`; `touch`; `STATUS:COMPLETED`;
    `PERCENT-COMPLETE:100` and `COMPLETED:<now UTC>` after `STATUS`; `RRULE` removed; JSON
    `RecurrenceValue`, `RecurrenceType`, `RecurrenceFrom` = 0; `RUID` kept, or set to the original
    UID if it is missing. Every other property is copied as is.
  - `next_due` adds `interval` months (monthly) or `12 × interval` months (yearly) with `NaiveDate::checked_add_months`. Any other shape → `Unsupported("repeat rule shape not verified: <rrule>")`. Callers have already excluded days 29–31 and Feb 29 through the gate (Task 9). A `next_due` whose day differs from the input day is a bug: return `Unsupported` rather than clamp.

- [ ] **Step 1: Write the failing tests:**
  - `one_time_complete_and_reopen_match_2do`: Task 2 `one-time` 04→05 and 05→06, changed-line sets and values as in Task 5.
  - `monthly_copy_and_advance_matches_2do`: input `rule-months-day-1-28/01-create/5c07…ics`, `now` = 2026-11-01T12:16:50Z, `copy_uid` = `6f19ef6b404644a38451520da5804e9f`. The live result differs from the input only in `DTSTAMP`, `LAST-MODIFIED`, `DTSTART`, `DUE`, with the 02-complete values (`20261210`). The copy has the same property multiset (name, params, value) as `02-complete/6f19…ics`, except `COMPLETED` = `now` (2Do's own completion time is earlier than its sync stamp).
  - `yearly_copy_and_advance_matches_2do`: same for `rule-years` (due `20271115`).
  - `next_due_intervals`: monthly 2026-12-10 ×2 → 2027-02-10; yearly 2026-11-15 ×2 → 2028-11-15; `FREQ=MONTHLY;BYMONTHDAY=10` → `Unsupported`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test edit_complete`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): one-time completion and 2Do copy-and-advance for verified repeats`.

### Task 7: Create

**Files:**
- Create: `crates/twydo-core/src/edit/create.rs`
- Modify: `crates/twydo-core/Cargo.toml` (`cargo add uuid --features v4 -p twydo-core`)
- Test: `crates/twydo-core/tests/edit_create.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn new_uid() -> String;                                   // uuid v4, 32 lowercase hex, no dashes (as 2Do)
  pub fn new_task(t: &NewTask, uid: &str, now: DateTime<Utc>) -> Result<Calendar, EditError>;
  pub fn file_name(uid: &str) -> String;                        // "<uid>.ics"
  ```
  Layout follows H10's evidence resource, in its property order, with these differences:
  `PRODID:-//Twydo//tdo <crate version>//EN`; no `X-2DOAPP-SYNC-*` properties (their meaning is
  unknown); CRLF line endings. `DESCRIPTION` is written empty when `None` (as 2Do does).
  Metadata is `default_task_meta(uid)`, plus `StartDate` when `start` is set. `DTSTART` mirrors
  `DUE`. `CREATED`, `DTSTAMP` and `LAST-MODIFIED` = `now`. Empty title → `EmptyTitle`.

- [ ] **Step 1: Write the failing tests:**
  - `new_uid_shape`: 32 chars, all `[0-9a-f]`, two calls differ.
  - `new_task_matches_2do_layout`: `NewTask { summary: "Test one-time", due: Some(Date(2026-12-03)), .. }` gives the same property names in the same order as the one-time `01-create` evidence, minus `X-2DOAPP-SYNC-*`. Metadata decodes equal to `default_task_meta(uid)`.
  - `new_task_reads_back`: `tasks_from_calendar(parse(serialize(new_task(..))))` gives the summary, the due date, `start` from `StartDate`, `kind == Task`, and no warnings.
  - `new_task_empty_title_rejected`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test edit_create`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): create tasks in 2Do's resource layout`.

### Task 8: Atomic file I/O, vdir lock, operation journal and recovery

**Files:**
- Create: `crates/twydo-core/src/store/{mod.rs,fsio.rs,journal.rs}` (`pub mod store` in `lib.rs`), `docs/journal.md`
- Test: `crates/twydo-core/tests/journal.rs`

**Interfaces:**
- Produces:
  ```rust
  // fsio.rs
  pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()>;  // ".<name>.tdo-tmp" in the same dir, fsync file, rename, fsync dir
  pub fn remove_durable(path: &Path) -> io::Result<()>;              // remove (missing = Ok), fsync dir
  pub struct VdirLock(File);
  impl VdirLock { pub fn exclusive(state_dir: &Path) -> io::Result<VdirLock>; pub fn shared(state_dir: &Path) -> io::Result<VdirLock>; } // "<state>/vdir.lock"
  // journal.rs
  #[derive(Serialize, Deserialize)] pub struct Entry {
      pub format: u32,                  // 1
      pub id: String,                   // "<YYYYMMDDTHHMMSSZ>-<8 hex>"
      pub created: DateTime<Utc>,
      pub kind: String,                 // "create" | "edit" | "complete" | "reopen" | "move" | "delete" | "undo" | "redo"
      pub uids: Vec<String>,
      pub stage: Stage,                 // Prepared | Applied | Abandoned   (serde lowercase)
      pub inverse_of: Option<String>,
      pub writes: Vec<Write>,
  }
  #[derive(Serialize, Deserialize)] pub struct Write { pub path: String /* relative to vdir */, pub before: Option<String>, pub after: Option<String> }  // None = file absent
  pub struct Journal { dir: PathBuf }   // "<state>/journal/", one "<id>.json" per entry, written with write_atomic
  impl Journal {
      pub fn open(state_dir: &Path) -> io::Result<Journal>;
      pub fn prepare(&self, e: &Entry) -> io::Result<()>;
      pub fn set_stage(&self, id: &str, s: Stage) -> io::Result<()>;
      pub fn get(&self, id: &str) -> io::Result<Option<Entry>>;
      pub fn list(&self) -> io::Result<Vec<Entry>>;            // sorted by id
  }
  pub fn apply_entry(vdir: &Path, e: &Entry) -> Result<(), ApplyError>; // per write: current==after → skip; current==before → write after; else Conflict{path}
  pub struct RecoveryReport { pub resumed: Vec<String>, pub stuck: Vec<(String, String /*reason*/)> }
  pub fn recover(vdir: &Path, state_dir: &Path) -> io::Result<RecoveryReport>; // no-op when "<state>/journal" is absent; takes the exclusive lock
  ```
  The protocol, documented in `docs/journal.md` together with the JSON shape: take the exclusive lock → `prepare` (Prepared) → `apply_entry` → `set_stage(Applied)` → release. Recovery rolls every Prepared entry forward with `apply_entry`. On a `Conflict` the entry stays Prepared and is reported as stuck. Applied entries are kept: pruning after sync confirmation is Plan 3's job. Writes are ordered so a history copy is created before its live task advances.

- [ ] **Step 1: Write the failing tests** (tempdirs for vdir and state):
  - `write_atomic_replaces_and_leaves_no_temp`; `remove_durable_missing_is_ok`.
  - `apply_entry_writes_creates_and_removes`.
  - `recover_rolls_forward_half_applied`: a Prepared entry with two writes, where the first is already applied on disk → after `recover`, both files equal `after`, the stage is Applied, and `resumed == [id]`.
  - `recover_is_idempotent`: a second `recover` changes nothing and resumes nothing.
  - `recover_reports_conflict_as_stuck`: the target file holds a third content → stuck with the path in the reason; the file is unchanged.
  - `recover_without_journal_dir_is_noop`: does not create `<state>/journal`.
  - `exclusive_lock_excludes_shared`: while `exclusive` is held in one thread, `File::try_lock_shared` on the lock file in another returns `WouldBlock`.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test journal`. Expected: FAIL.
- [ ] **Step 3:** Implement and write `docs/journal.md`.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): atomic writes, vdir lock and recoverable operation journal`.

### Task 9: The write gate

**Files:**
- Create: `crates/twydo-core/src/gate.rs`
- Test: `crates/twydo-core/tests/gate.rs`

**Interfaces:**
- Consumes: `Matrix`, `combined_verdicts`, `WriteMode` (Task 1); `Op`, `Change` (Task 5); `Snapshot`, `rule_class`, `parse_rrule`.
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)] pub struct Blocked(pub String);   // Display = the reason
  pub fn check(snap: &Snapshot, m: &Matrix, mode: WriteMode, op: &Op, today: NaiveDate) -> Result<(), Blocked>;
  pub fn task_case(snap: &Snapshot, idx: usize) -> Result<&'static str, Blocked>;
  ```
  Rules, applied in this order. The first failure wins, and reason strings are exact:
  1. Target lookup: unknown UID → `"no task with UID <uid>"`; a duplicate UID → `"duplicate UID"`; `task.blocked` → that reason; `Move` to, or `Create` in, an unknown collection → `"no list <id>"`.
  2. `task_case`: a project, checklist, task with a parent or task with children → `"projects and checklists cannot be edited yet"`. No RRULE → `one-time`. Class base `months-*` → `monthly`, `years*` → `yearly`, any `+from-completion` → `"repeat from completion is not verified yet"`. Other classes → `"<class> repeats are not verified yet"`.
  3. Matrix cases for the op. `Create` → `one-time` + the attribute cases of the new task. `Edit` → the task case, plus attribute cases for `Due` (`date-only` and/or `timed`, before and after) and `Start` (`start-date`). Other ops → the task case only. Each case/op needs `iphone_baseline == Pass`. In `Verified` mode it also needs `twydo_to_iphone == Pass && iphone_to_twydo == Pass`. Failure → `"<op> for <case> tasks is not verified yet (<column>)"`, e.g. `"complete for timed tasks is not verified yet (iphone_baseline)"`.
  4. Recurring tasks: any edit needs `preserve == Pass` (combined verdicts). `RepeatInterval` needs `edit == Pass`. `Complete` needs `execute == Pass`. Removing the due date of a recurring task → `"removing the due date of a repeating task is not verified yet"`.
  5. Recurring `Complete` shape (Task 6 `next_due` shape). It also needs: a date-only due, no start (`"completing a repeating task with a start date is not verified yet (start-due-alarm-offsets)"`), no alarms (same edge-case id), and `next_due > today` (else `"completing a repeating task after a missed occurrence is not verified yet (monthly-15th-three-months-late)"`).
  6. `Reopen` of a completed history copy (JSON `RUID` ≠ UID) → `"reopening a repeat's history copy is not verified yet"` unless `monthly`/`yearly` reopen has `iphone_baseline == Pass`.
  Edge-case verdicts are never relaxed by `Test` mode. Mode only changes rule 3.

- [ ] **Step 1: Write the failing tests** with a synthetic snapshot (built from small `.ics` strings in `tests/data/gate/`) and a test matrix (TOML string), `today` = 2026-11-04:
  - `verified_mode_blocks_without_twydo_columns`; `test_mode_allows_with_baseline_only`.
  - `edit_due_adds_timed_case`: a date-only → timed edit is blocked when `timed`/`edit` has no baseline.
  - `project_and_checklist_blocked`; `from_completion_blocked`; `weekly_blocked` (`"weeks repeats are not verified yet"`).
  - `late_recurring_completion_blocked`: monthly due 2026-08-15 → the missed-occurrence reason.
  - `recurring_with_start_or_alarm_blocked`; `recurring_due_removal_blocked`; `monthly_day_31_blocked` (class `months-day-29-31` → `"months-day-29-31 repeats are not verified yet"`).
  - `history_copy_reopen_blocked`.
  - `embedded_matrix_blocks_everything_in_verified_mode`: with `Matrix::embedded()` before Task 12, a one-time `Complete` is blocked.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test gate`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p twydo-core`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): write gate driven by the compatibility matrix`.

### Task 10: Store — apply, undo, redo

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs`, `crates/twydo-core/src/vdir.rs`, `crates/twydo-core/src/views.rs`
- Test: `crates/twydo-core/tests/store.rs`

**Interfaces:**
- Consumes: Tasks 5–9.
- Produces:
  ```rust
  // vdir.rs / views.rs
  pub struct LoadReport { …, pub sources: HashMap<PathBuf, Vec<u8>> }   // bytes of every loaded .ics
  impl Snapshot { pub fn source(&self, path: &Path) -> Option<&[u8]>; pub fn find(&self, uid: &str) -> Option<usize>; }
  // store/mod.rs
  #[derive(thiserror::Error, Debug)] pub enum WriteError {
      #[error("{0}")] Blocked(Blocked),
      #[error("{0}")] Edit(#[from] EditError),
      #[error("{} changed on disk; reload and try again", .0.display())] Stale(PathBuf),
      #[error("journal has unfinished operations: {0}")] Stuck(String),
      #[error("nothing to undo")] NothingToUndo,
      #[error("nothing to redo")] NothingToRedo,
      #[error(transparent)] Io(#[from] io::Error),
  }
  pub struct Store { vdir: PathBuf, state: PathBuf, mode: WriteMode, matrix: Matrix, journal: Journal, done: Vec<String>, undone: Vec<String> }
  impl Store {
      pub fn open(cfg: &Config) -> Result<(Store, RecoveryReport), WriteError>;   // = open_with(cfg, Matrix::embedded().clone())
      pub fn open_with(cfg: &Config, matrix: Matrix) -> Result<(Store, RecoveryReport), WriteError>; // runs recovery; any stuck entry → later writes return Stuck
      pub fn apply(&mut self, snap: &Snapshot, op: &Op, now: DateTime<Utc>) -> Result<String /*op id*/, WriteError>;
      pub fn undo(&mut self, id: &str, now: DateTime<Utc>) -> Result<String, WriteError>;   // new entry: writes swapped, inverse_of = id
      pub fn undo_last(&mut self, now: DateTime<Utc>) -> Result<String, WriteError>;        // session stack (u)
      pub fn redo_last(&mut self, now: DateTime<Utc>) -> Result<String, WriteError>;        // session stack (Ctrl-r); a new apply clears it
      pub fn abandon(&mut self, id: &str) -> Result<(), WriteError>;                         // stuck entry → Abandoned (file kept for manual recovery)
  }
  ```
  `apply`: `gate::check` (today = `now` in the snapshot's tz). Under the exclusive lock, every
  target file must still equal `snap.source(path)`, else `Stale`. Build the writes: `Create` →
  `<collection>/<new_uid>.ics`. `Move` → the same bytes and file name in the destination (H5),
  then the source removed. `Delete` → the file removed. Complete recurring → copy first, then live.
  Then run the journal protocol (Task 8). `undo` of an entry requires every file to equal that
  entry's `after` (else `Stale`). Undo of a recurring completion additionally requires edge case
  `reopen-undo-before-after-sync` to be `Pass` in `Verified` mode.

- [ ] **Step 1: Write the failing tests** (tempdir vdir copied from `fixtures/phase0/v1/rule-months-day-1-28/01-create`, plus a synthetic one-time task; a test matrix with every needed baseline = `Pass`; mode `Test`):
  - `complete_recurring_writes_copy_and_advances_live`: two files afterwards, one Applied journal entry with 2 writes (copy first).
  - `unchanged_resources_untouched`: the other files keep their bytes and mtimes.
  - `apply_refuses_when_file_changed_since_load`: modify the file after `load` → `Stale`, no journal entry, file unchanged.
  - `undo_restores_bytes_exactly` then `redo_last_reapplies`; `new_apply_clears_redo`.
  - `undo_refuses_after_external_change` → `Stale`.
  - `move_keeps_file_name_and_bytes`; `delete_then_undo_restores_file`; `create_writes_new_uid_file`.
  - `blocked_op_writes_nothing`: verified-mode embedded matrix → `WriteError::Blocked`, vdir and journal unchanged.
  - `stuck_journal_blocks_writes`: a Prepared conflicting entry → `open` reports it stuck, `apply` → `Stuck`; after `abandon`, `apply` works.
- [ ] **Step 2:** Run `cargo test -p twydo-core --test store`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): store with journaled apply, undo and redo`.

### Task 11: `tdo op` CLI and recovery at startup

**Files:**
- Create: `crates/tdo/src/cli/op.rs`
- Modify: `crates/tdo/src/main.rs`, `crates/tdo/src/cli/mod.rs`, `crates/tdo/src/app.rs`, `docs/config.md`
- Test: `crates/tdo/tests/cli_op.rs`, `crates/tdo/tests/app_keys.rs`

**Interfaces:**
- Consumes: `Store`, `Op`, `Change`, `Due`, `NewTask`, `gate::check`, `recover`.
- Produces the command surface (exit 0 = done and prints the op id; 1 = blocked or stale, reason on stderr; 2 = usage/config/IO error):
  ```
  tdo op add --list <ID> --title <T> [--notes <N>] [--due <YYYY-MM-DD | YYYY-MM-DDTHH:MM>] [--start <YYYY-MM-DD>]
  tdo op edit <UID> [--title T] [--notes N | --clear-notes] [--due D | --clear-due] [--start D | --clear-start] [--every N]
  tdo op complete <UID> | reopen <UID> | delete <UID> | move <UID> --to <ID>
  tdo op undo <OP-ID>
  tdo op check <UID>          # one line per op kind: "complete: ok" or "complete: <reason>"
  tdo op journal              # id, created, kind, stage, uids — one per line
  tdo op recover              # runs recovery, prints resumed/stuck
  tdo op abandon <OP-ID>
  ```
  `tdo tui`, `export-json` and `check` call `store::recover` on the configured state dir before
  loading. A stuck entry prints `tdo: journal has unfinished operations; run 'tdo op journal'` and
  continues read-only. The TUI's disabled-key message becomes `"<action>: not available yet"`.

  `tdo op` takes a hidden `--matrix <PATH>` (clap `hide = true`) that replaces the embedded matrix, so tests do not depend on the matrix's current verdicts. It is a test hook, not a documented option.

- [ ] **Step 1: Write the failing tests** (via `std::process::Command` on the built binary, as in `tests/cli.rs`; `XDG_CONFIG_HOME` and `XDG_STATE_HOME` pointed at tempdirs; a vdir copied from the phase-0 fixtures; a test matrix written to a tempfile and passed with `--matrix`):
  - `op_blocked_in_verified_mode_exits_1`: `tdo op complete <uid>` on a copied fixture → exit 1, stderr has `not verified yet`, file unchanged.
  - `op_test_mode_warns`: stderr has `test write mode: unverified operations are enabled`.
  - `op_complete_in_test_mode_prints_op_id`: test matrix with the `monthly`/`complete` baseline = `pass`, mode `test` → exit 0, stdout matches `^\d{8}T\d{6}Z-[0-9a-f]{8}$`, and the vdir has the history copy.
  - `op_check_lists_every_kind`: six lines, prefixed `edit:`, `complete:`, `reopen:`, `move:`, `delete:`, `create:`.
  - `op_journal_and_recover_run_on_empty_state`: exit 0, `resumed: 0`, `stuck: 0`.
  - `recover_runs_before_export`: a Prepared journal entry under the state dir is applied before `export-json` prints (the exported title is the `after` title).
  - In `app_keys.rs`: update the gated-key expectation to `"done: not available yet"`.
- [ ] **Step 2:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 3:** Implement with clap subcommands. Parse dates with chrono; `--due` with a `T` is `Due::Time`.
- [ ] **Step 4:** Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(tdo): tdo op commands and journal recovery at startup`.

### Task 12: Verify Twydo writes against the iPhone (👤 user + 🤖, test account only)

**Files:**
- Modify: `docs/phase0/plan4-baseline.md`, `compat/matrix.toml`, `docs/phase0/README.md` (short Result note), `fixtures/phase0/v1/…` (twydo steps)

**Stop and confirm with the user** before starting: same test setup as Task 2, with a Twydo config
whose `vdir` is the test vdir, `[writes] mode = "test"`, and its own `state_dir`.

- [ ] **Step 1:** Append to `plan4-baseline.md` a checklist that runs each enabled operation with `tdo op` (scenarios `twydo-one-time`, `twydo-date-only`, `twydo-timed`, `twydo-start-date`, `twydo-monthly`, `twydo-yearly`). Each operation is followed by: `vdirsyncer sync twydo`, capture with `--device twydo`, a 👤 check on the iPhone (title, dates, start, repeat and completion state appear as intended), then a 👤 edit of the same task on the iPhone, sync, capture, and a 🤖 `tdo export-json` check that Twydo reads the iPhone's edit correctly. Include undo of a monthly completion twice: once before `vdirsyncer sync` and once after it (edge case `reopen-undo-before-after-sync`).
- [ ] **Step 2 (👤+🤖):** Run it. Scan, user-confirm, and commit fixtures as in Task 2.
- [ ] **Step 3:** Set `twydo_to_iphone` / `iphone_to_twydo` to `pass` or `fail` per case/op with evidence. Set the edge case to `pass` only if both undo runs left the iPhone with the right live date and no stray history copy. Record anything surprising in `plan4-baseline.md`. A `fail` blocks that op in `verified` mode automatically. Note in the issue tracker (`gh issue create`) any fix that needs code.
- [ ] **Step 4:** Run `cargo test --workspace`, then `tdo op check <uid>` with `[writes] mode = "verified"` on the test vdir. Expected: tests PASS; the verified ops print `ok`.
- [ ] **Step 5:** Commit: `chore(compat): verify Twydo writes against iPhone 2Do`. Update the memory note / AGENTS.md "What to do next" to point at Plan 5.
