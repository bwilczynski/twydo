# Twydo Plan 1: Phase-0 kit and read-only foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `tdo` binary that reads a vdir losslessly and shows Today / All / Scheduled / Done / Next 3 Days / lists in the three-pane TUI, plus the tooling the user needs to run the phase-0 compatibility gate. Twydo writes no task data in this plan.

**Architecture:** A Cargo workspace with two crates. `twydo-core` (library): a lossless iCalendar content-line layer, a task model on top of it, a vdir loader, local config, view/grouping logic, and the compatibility-matrix loader. `tdo` (binary): CLI subcommands and the ratatui TUI. The phase-0 helpers are stdlib-only Python scripts in `tools/`, next to the existing decoder.

**Tech Stack:** Rust (edition 2024), `chrono` + `chrono-tz` + `iana-time-zone`, `serde`/`toml`/`serde_json`, `thiserror`, `regex`, `clap` (derive), `ratatui` (crossterm backend), `notify` + `notify-debouncer-mini`, `unicode-width`, `tempfile` (dev). Python 3 stdlib + `unittest` for tools (pytest isn't installed). Add crates with `cargo add` at their current versions.

**Spec:** `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` (with `docs/design/tui-layout.md` and `docs/reference/2do/`).

## Plan series (spec breakdown)

The spec is too broad for one plan. Each plan below produces working, testable software.

| # | Plan | Spec | Depends on |
|---|---|---|---|
| **1** | **This plan:** phase-0 kit, read-only core, read-only TUI | §6 (reading), §7 views, §8, §10 steps 0–2 | — |
| — | **Phase-0 gate, run by the user** using this plan's kit: backups, NAS test account, fixtures, matrix | §6, §10 step 0 | Plan 1 tasks 8–11 |
| 2 | Notifier: `tdo notify`, delivery state, catch-up/summaries, systemd timer + launchd agent | §5 notifications, §7 delivery contract, §9 notifier | Plan 1; alarm fixtures from the gate before release |
| 3 | Sync runner and conflicts: vdirsyncer + metasync runner, status, backoff, conflict store, `:sync`, `:conflicts` | §5 storage/conflicts, §9 failures | gate (vdirsyncer setup verified) |
| 4 | Writable core: preserving edits, metadata counterparts, operation journal, undo/redo, completion and recurrence per the verified matrix | §6 preservation/recurring completion/recovery | gate outputs |
| 5 | TUI editing and natural-language add (`o`), date shortcuts, move, delete, manual order, list management | §7 creation/editing, §8 keys | Plan 4 |
| 6 | Interop and production migration: §9 interop on the test setup, reconciliation, P1/P2/L1, cutover | §9, §10 step 6 | Plans 2–5 |
| 7 | Later: global quick-entry (`tdo add`) | §10 step 7 | Plan 5 |

## Global Constraints

- App name **Twydo**; command **`tdo`**; config at **`~/.config/twydo/config.toml`** on both Linux and macOS (honour `$XDG_CONFIG_HOME` when set; do not use macOS `Library/Application Support`).
- **Twydo writes no task resource in this plan.** Keys and commands that would write (`x`, `Space`, `o`, `O`, `i`, `J`, `K`, `dd`, `u`, `Ctrl-r`, `t`, `T`, `]`, `[`, `}`, `{`, `r`, `a`, `m`, `:sync`, `:conflicts`) show the reason `"<action>: not enabled until the phase-0 compatibility gate passes"`.
- Reading never modifies a file: no rewrites, no timestamp changes, no normalisation.
- UI chrome uses only the 16 ANSI colours (`ratatui::style::Color` named variants and `Reset`). True colour (`Color::Rgb`) is allowed **only** for list colour bars, taken from the collection's `color` file.
- **No personal data in the repo** (AGENTS.md). Test data is synthetic. `.ics` files are gitignored; only `crates/twydo-core/tests/data/**` and `fixtures/phase0/**` get exceptions, and phase-0 fixtures are committed only after `tools/capture_fixture.py scan` passes and the user confirms the content is synthetic.
- Timezones in tests use `America/New_York` (synthetic choice). The reference "now" in view tests is **Wed 2026-11-04 10:00 America/New_York** unless a test says otherwise.
- Do not substitute library defaults for unresolved 2Do behaviour (spec §6). Where this plan reads 2Do metadata whose encoding is **[?]**, it records a warning on the task rather than guessing silently.
- Ask the user before any action that touches their production 2Do data, NAS or iPhone.
- Commit messages have no agent attribution (user's global instructions).

## Review Focus

1. **A broken or non-task `.ics` in the vdir** (vdirsyncer mid-write, a VEVENT-only file, a truncated file): load everything else, list the bad file in the load report and `tdo check`, never crash. Test → Task 6 (`load_skips_bad_files_and_reports_them`).
2. **The date rolls over while the TUI is open** (midnight, or a laptop waking from sleep): Today, the groups and the relative dates must recompute without a restart. Test → Task 12 (`tick_recomputes_view_on_new_day`).
3. **A TZID chrono-tz doesn't know** (e.g. a Windows name like `W. Europe Standard Time`, or a custom VTIMEZONE): the task still loads and shows, falling back to local time with a warning. Test → Task 3 (`unknown_tzid_falls_back_to_local_with_warning`).
4. **A broken parent link** (`RELATED-TO` names a missing UID, a UID in another list, or there is a cycle): show the task at the top level with a warning; no infinite loop. Test → Task 7 (`orphan_and_cyclic_parents_render_top_level`).
5. **Long or wide titles in a narrow terminal** (CJK, emoji): truncate by display width so the date column stays aligned. Test → Task 13 (`wide_titles_truncate_by_display_width`).

---

## File structure

```
Cargo.toml                        workspace
rust-toolchain.toml, mise.toml    toolchain pin
crates/twydo-core/
  src/lib.rs
  src/ical/mod.rs                 Calendar/Component/Property, parse, serialize
  src/ical/value.rs               date-time values, durations, text unescape
  src/model/task.rs               Task, Status, Alarm, TaskKind, from_component
  src/model/meta.rs               X-2DOAPP-METADATA @key:value parsing
  src/model/repeat.rs             RRULE parse, describe, short_hint, rule_class
  src/vdir.rs                     Collection, LoadReport, load()
  src/config.rs                   Config, ListPrefs, Group, Sort
  src/views.rs                    Snapshot, View, DateGroup, ViewResult, counts
  src/format.rs                   relative_date, detail strings
  src/compat.rs                   compatibility matrix loader
  tests/data/                     synthetic .ics and sample vdir
crates/tdo/
  src/main.rs                     clap CLI
  src/cli/export.rs, cli/check.rs
  src/app.rs                      App state + key handling (no terminal)
  src/ui/mod.rs, ui/sidebar.rs, ui/tasks.rs, ui/details.rs, ui/status.rs
  src/term.rs                     terminal loop, watcher, minute tick
compat/matrix.toml                compatibility matrix (template, filled during phase 0)
docs/config.md                    config reference
docs/phase0/README.md             gate checklist (user + agent steps)
contrib/vdirsyncer/config.example
tools/capture_fixture.py, tools/reconcile.py, tools/tests/
```

---

### Task 1: Workspace scaffold, toolchain, rename to Twydo

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `mise.toml`, `crates/twydo-core/Cargo.toml`, `crates/twydo-core/src/lib.rs`, `crates/tdo/Cargo.toml`, `crates/tdo/src/main.rs`
- Modify: `.gitignore`, `README.md`, `AGENTS.md` (replace the placeholder `tick` with Twydo / `tdo`, per spec §4.8; describe the new layout)

**Interfaces:**
- Produces: crate `twydo_core`; binary `tdo` with clap subcommands `tui` (the default when none is given), `export-json`, `check`, and a global `--vdir <PATH>` flag. Subcommands print `not implemented` for now.

- [ ] **Step 1:** Pin the toolchain: `rust-toolchain.toml` with `channel = "1.99.0"`, components `rustfmt`, `clippy`; and `mise use rust@1.99.0` (writes `mise.toml`). Verify with `cargo --version`. Expected: `cargo 1.99.0`.
- [ ] **Step 2:** Create the workspace (`resolver = "3"`, `edition = "2024"` in `[workspace.package]`) and both crates. Add a smoke test in `crates/tdo/tests/cli.rs`, `version_flag_prints_name`: run the built binary (`env!("CARGO_BIN_EXE_tdo")`) with `--version` and assert stdout starts with `tdo `.
- [ ] **Step 3:** Append to `.gitignore`:
  ```
  # Synthetic test data and vetted phase-0 fixtures only
  !/crates/twydo-core/tests/data/**/*.ics
  !/fixtures/phase0/**/*.ics
  ```
  Check: `git check-ignore -v crates/twydo-core/tests/data/x.ics` prints nothing; `git check-ignore -v foo.ics` still matches.
- [ ] **Step 4:** Run `cargo test --workspace && cargo clippy --workspace -- -D warnings`. Expected: pass, no warnings.
- [ ] **Step 5:** Commit: `chore: cargo workspace scaffold; rename tick to Twydo`.

### Task 2: Lossless iCalendar content-line layer

**Files:**
- Create: `crates/twydo-core/src/ical/mod.rs`, `crates/twydo-core/tests/ical_roundtrip.rs`, `crates/twydo-core/tests/data/ical/*.ics` (synthetic)

**Interfaces:**
- Produces:
  ```rust
  pub struct Calendar { pub root: Component }            // the VCALENDAR
  pub struct Component { pub name: String, pub props: Vec<Property>, pub children: Vec<Component>,
                         raw_begin: Vec<u8>, raw_end: Vec<u8> }
  pub struct Property { pub name: String, pub params: Vec<Param>, pub value: String, raw: Vec<u8> }
  pub struct Param { pub name: String, pub values: Vec<String> }
  pub fn parse(bytes: &[u8]) -> Result<Calendar, ParseError>;
  pub fn serialize(cal: &Calendar) -> Vec<u8>;
  impl Component { pub fn prop(&self, name: &str) -> Option<&Property>;
                   pub fn props_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item=&'a Property>;
                   pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item=&'a Component>; }
  impl Property { pub fn param(&self, name: &str) -> Option<&str>; }   // first value, case-insensitive name
  #[derive(thiserror::Error)] pub enum ParseError { #[error("line {line}: {msg}")] At { line: usize, msg: String } }
  ```
  Names are compared case-insensitively and stored uppercased; `raw` keeps the original bytes exactly, including folds and line endings.

- [ ] **Step 1: Write the failing tests**, each reading a synthetic file and asserting `serialize(&parse(b)?) == b`:
  - `roundtrip_crlf_folded` (75-octet folds with a space continuation)
  - `roundtrip_fold_inside_utf8_sequence` (a fold that splits a multi-byte character; unfolding works on bytes, then decodes; assert `value == "Zażółć gęślą jaźń 🙂"`)
  - `roundtrip_lf_only_and_tab_continuation`
  - `roundtrip_unknown_props_params_components` (`X-2DOAPP-METADATA`, `X-FOO;X-BAR="a:b;c,d":v`, a `VTIMEZONE`, a nested `VALARM`, an unknown `X-THING` component)
  - `quoted_param_values_parse`: `param("X-BAR") == Some("a:b;c,d")`
  - `missing_end_is_error`: `parse` returns `ParseError::At { line: 5, .. }` for a file truncated after line 5
- [ ] **Step 2:** Run `cargo test -p twydo-core --test ical_roundtrip`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement `parse`/`serialize` in `ical/mod.rs`. Hand-written on purpose: library parsers normalise, and spec §9 needs unchanged resources to stay byte-equal. Split into physical lines while keeping the terminators, group continuation lines into logical lines that keep their raw bytes, then parse `name *(";" param) ":" value` honouring DQUOTE. `serialize` writes `raw` for every property and begin/end line. (Re-emitting edited properties belongs to Plan 4.)
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): lossless iCalendar content-line parser`.

### Task 3: Date-time values, durations, text

**Files:**
- Create: `crates/twydo-core/src/ical/value.rs`, `crates/twydo-core/tests/ical_values.rs`

**Interfaces:**
- Consumes: `Property` (Task 2).
- Produces:
  ```rust
  pub enum IcalDateTime { Date(NaiveDate), Floating(NaiveDateTime), Utc(DateTime<Utc>),
                          Zoned { local: NaiveDateTime, tzid: String } }
  pub enum Resolved { Date(NaiveDate), Instant(DateTime<Tz>) }
  pub fn parse_date_time(p: &Property) -> Result<IcalDateTime, ValueError>;
  impl IcalDateTime { pub fn resolve(&self, local: Tz) -> (Resolved, Option<String> /*warning*/);
                      pub fn local_date(&self, local: Tz) -> NaiveDate; pub fn is_date_only(&self) -> bool; }
  pub fn parse_duration(s: &str) -> Result<chrono::Duration, ValueError>;   // RFC 5545 dur-value
  pub fn unescape_text(s: &str) -> String;                                  // \n \N \, \; \\
  ```
  Floating and zoned values resolve in `local` or `tzid` respectively. A nonexistent local time (DST gap) moves forward to the first valid instant; an ambiguous one (fall-back) takes the earlier offset.

- [ ] **Step 1: Write the failing tests:**
  - `date_value`: `DUE;VALUE=DATE:20261104` → `Date(2026-11-04)`
  - `utc_floating_zoned`: `…T150000Z` → `Utc`; `…T150000` → `Floating`; `DUE;TZID=America/New_York:20261104T150000` → `Zoned`, resolves to `2026-11-04T15:00-05:00`
  - `dst_gap_moves_forward`: zoned `20260308T023000` NY → `03:30-04:00`
  - `dst_ambiguous_takes_earlier`: zoned `20261101T013000` NY → `01:30-04:00`
  - `unknown_tzid_falls_back_to_local_with_warning`: `TZID=W. Europe Standard Time` resolves in the `local` tz and the warning contains `unknown TZID`
  - `durations`: `-PT15M` = −15 min; `P1W` = 7 days; `-P1DT2H` = −26 h; `PT0S` = 0
  - `unescape`: `a\,b\;c\nd\\` → `"a,b;c\nd\\"`
- [ ] **Step 2:** Run `cargo test -p twydo-core --test ical_values`. Expected: FAIL.
- [ ] **Step 3:** Implement in `ical/value.rs` with `chrono-tz` for TZID lookup.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): iCalendar date-time, duration and text values`.

### Task 4: Task model and 2Do metadata (read side)

**Files:**
- Create: `crates/twydo-core/src/model/{mod.rs,task.rs,meta.rs}`, `crates/twydo-core/tests/model_task.rs`, synthetic `.ics` under `tests/data/model/`

**Interfaces:**
- Consumes: Tasks 2–3.
- Produces:
  ```rust
  pub enum TaskKind { Task, Project, Checklist }
  pub enum Status { Open, Completed, Cancelled }
  pub enum Trigger { Relative { offset: Duration, related_end: bool }, Absolute(DateTime<Utc>) }
  pub struct Alarm { pub trigger: Trigger, pub index: usize }  // index = position among the VTODO's VALARMs
  pub struct Metadata { pub raw: Option<String>, pub tokens: BTreeMap<String, Option<String>> }
  pub struct Task {
      pub uid: String, pub collection: String, pub path: PathBuf,
      pub summary: String, pub description: Option<String>,
      pub due: Option<IcalDateTime>, pub start: Option<IcalDateTime>,
      pub status: Status, pub completed: Option<IcalDateTime>,
      pub created: Option<IcalDateTime>, pub last_modified: Option<IcalDateTime>,
      pub rrule: Option<String>,               // raw value; parsed in Task 5
      pub parent: Option<String>, pub kind: TaskKind, pub alarms: Vec<Alarm>,
      pub meta: Metadata, pub from_completion: bool,
      pub warnings: Vec<String>, pub blocked: Option<String>,   // reason edits must stay off
  }
  pub fn tasks_from_calendar(cal: &Calendar, collection: &str, path: &Path) -> Vec<Task>;
  pub fn parse_metadata(raw: &str) -> Metadata;
  ```
  Rules (read side only; precedence is for phase 0 to settle, spec §6):
  - `parent` = `RELATED-TO` with no `RELTYPE` or `RELTYPE=PARENT`; other reltypes are ignored.
  - `kind` comes from `@tasktype:` (`project`/`checklist`), otherwise `Task`. 2Do's encoding is **[?]**: if metadata is absent but the task has children, `views` treats it as a project for display only and adds the warning `"parent type not encoded; shown as project"`.
  - `from_completion` = the `@repeatfromcompletion` token is present.
  - `start` = `DTSTART` when present. Otherwise use metadata `@startdate`/`@starttime`. When both are present and disagree, add the warning `"start date: DTSTART and 2Do metadata disagree"` and use `DTSTART`. (Phase 0 found that 2Do's CalDAV metadata is `<2Do Meta>` JSON whose `StartDate` alone decides the start, since 2Do's `DTSTART` copies `DUE`; see 06 §2.6.)
  - `Completed` if `STATUS:COMPLETED` or a `COMPLETED` property is present; `Cancelled` if `STATUS:CANCELLED`.
  - A `RECURRENCE-ID`, or more than one VTODO per resource: take the VTODO without `RECURRENCE-ID` as the master and set `blocked = "recurrence overrides are not supported"`.
  - VEVENT/VJOURNAL components are ignored.
  - `parse_metadata` tokenises `@key` or `@key:value`. A value runs until whitespace followed by `@<letters>` (followed by `:`, whitespace or end of text), so `@tags:a, b` keeps `a, b`.

- [ ] **Step 1: Write the failing tests:**
  - `basic_fields`: synthetic one-time task → summary, description unescaped, `due` date-only, `Open`
  - `parent_and_kind`: child with `RELATED-TO;RELTYPE=PARENT:p1`; parent with `X-2DOAPP-METADATA:@tasktype:checklist @repeatfromcompletion` → `kind == Checklist`, `from_completion`
  - `metadata_tokens`: `parse_metadata("@taskid:abc @tags:a, b @starred @duetime:09:30")` → `tags = Some("a, b")`, `starred = None` (present), `duetime = Some("09:30")`
  - `start_conflict_warns`: `DTSTART;VALUE=DATE:20261102` + `@startdate:2026-11-03` → `start` is 11-02 and the warning is present
  - `alarms`: `TRIGGER:-PT15M` → `Relative{-15m, related_end:false}`; `TRIGGER;RELATED=END:PT0S` → `related_end:true`; `TRIGGER;VALUE=DATE-TIME:20261104T140000Z` → `Absolute`
  - `recurrence_override_blocks`: two VTODOs with the same UID, one with `RECURRENCE-ID` → one Task, `blocked.is_some()`
  - `completed_and_cancelled`: status mapping
- [ ] **Step 2:** Run `cargo test -p twydo-core --test model_task`. Expected: FAIL.
- [ ] **Step 3:** Implement per the rules above.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): read VTODO tasks and 2Do metadata`.

### Task 5: Repeat rules — parse, describe, classify

**Files:**
- Create: `crates/twydo-core/src/model/repeat.rs`, `crates/twydo-core/tests/repeat.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum Freq { Daily, Weekly, Monthly, Yearly }
  pub enum End { Never, Count(u32), Until(IcalDateTime) }
  pub struct RRule { pub freq: Freq, pub interval: u32, pub by_day: Vec<(Option<i8>, Weekday)>,
                     pub by_month_day: Vec<i8>, pub by_month: Vec<u8>, pub by_set_pos: Vec<i8>,
                     pub end: End, pub unknown: Vec<(String, String)> }
  pub fn parse_rrule(s: &str) -> Result<RRule, RepeatError>;
  pub fn describe(r: &RRule, anchor: Option<NaiveDate>, from_completion: bool) -> String;
  pub fn short_hint(r: &RRule) -> String;
  pub fn rule_class(r: &RRule, anchor: Option<NaiveDate>, from_completion: bool) -> String;
  ```
  This plan does **not** execute rules; that waits for the gate (Plan 4).

- [ ] **Step 1: Write the failing table test** `describe_and_classify` (columns: rrule, anchor, from_completion → describe | short_hint | rule_class):

  | RRULE | anchor | fc | describe | hint | class |
  |---|---|---|---|---|---|
  | `FREQ=DAILY` | – | n | `Daily` | `daily` | `days` |
  | `FREQ=DAILY;INTERVAL=3` | – | n | `Every 3 Days` | `every 3d` | `days` |
  | `FREQ=WEEKLY;BYDAY=MO,TH` | – | n | `Every Mon, Thu` | `weekly` | `weekdays-set` |
  | `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR,SA,SU` | – | n | `Every Day` | `daily` | `weekdays-set` |
  | `FREQ=WEEKLY;INTERVAL=2` | – | n | `Bi-weekly` | `every 2w` | `weeks` |
  | `FREQ=MONTHLY` | 2026-11-25 | n | `Monthly on the 25th` | `monthly` | `months-day-1-28` |
  | `FREQ=MONTHLY` | 2026-10-31 | n | `Monthly on the 31st` | `monthly` | `months-day-29-31` |
  | `FREQ=MONTHLY;INTERVAL=6` | 2026-11-01 | n | `Semi-yearly on the 1st` | `every 6m` | `months-day-1-28` |
  | `FREQ=MONTHLY;BYDAY=2TU` | – | n | `The second Tue of each month` | `monthly` | `months-nth-weekday` |
  | `FREQ=MONTHLY;BYMONTHDAY=-1;INTERVAL=2` | – | n | `On the last day of every 2 months` | `every 2m` | `months-last-day` |
  | `FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1` | – | n | `The last weekday of each month` | `monthly` | `months-nth-workday` |
  | `FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1` | – | n | `The first weekend day of each month` | `monthly` | `months-nth-weekend-day` |
  | `FREQ=YEARLY` | 2028-02-29 | n | `Yearly on 29 Feb` | `yearly` | `years-feb-29` |
  | `FREQ=MONTHLY;COUNT=3` | 2026-11-15 | y | `Monthly on the 15th, from completion, 3 times` | `monthly` | `months-day-1-28+from-completion+count` |
  | `FREQ=YEARLY;UNTIL=20271231` | 2026-11-15 | n | `Yearly on 15 Nov, until 31 Dec 2027` | `yearly` | `years+until` |
  | `FREQ=MONTHLY;BYHOUR=9` | – | n | `Monthly (unrecognised parts: BYHOUR=9)` | `monthly` | `unrecognized` |

  Labels follow 2Do's (01 §6): `Bi-monthly` for interval 2, `Every N Months` otherwise, `Semi-yearly` for 6. Also `parse_errors`: `FREQ=HOURLY` → `Err` (outside 2Do's vocabulary; the task then shows `repeat: unsupported rule` and is blocked).
- [ ] **Step 2:** Run `cargo test -p twydo-core --test repeat`. Expected: FAIL.
- [ ] **Step 3:** Implement. `rule_class` modifiers are appended in the order `+from-completion`, then `+count` or `+until`.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): parse and describe 2Do repeat rules`.

### Task 6: Vdir loader and config

**Files:**
- Create: `crates/twydo-core/src/vdir.rs`, `crates/twydo-core/src/config.rs`, `crates/twydo-core/tests/vdir.rs`, `crates/twydo-core/tests/config.rs`, `crates/twydo-core/tests/data/sample-vdir/**` (3 synthetic collections, ~15 tasks covering every row type in the layout doc, placeholder titles), `docs/config.md`

**Interfaces:**
- Consumes: `tasks_from_calendar` (Task 4).
- Produces:
  ```rust
  pub struct Collection { pub id: String, pub path: PathBuf, pub display_name: Option<String>, pub color: Option<(u8,u8,u8)> }
  pub struct LoadReport { pub collections: Vec<Collection>, pub tasks: Vec<Task>, pub errors: Vec<(PathBuf, String)> }
  pub fn load(root: &Path) -> io::Result<LoadReport>;   // Err only if root is unreadable
  pub fn parse_color(s: &str) -> Option<(u8,u8,u8)>;    // "#RRGGBB" or "#RRGGBBAA"
  #[serde(deny_unknown_fields)] pub struct Config {
      pub vdir: PathBuf, pub timezone: Option<String>, pub week_start: Weekday /*default Mon*/,
      pub morning_time: NaiveTime /*09:00*/, pub default_list: Option<String>,
      pub lists: BTreeMap<String, ListPrefs>, pub groups: Vec<Group> }
  pub struct ListPrefs { pub archived: bool, pub hidden: bool, pub show_in_all: bool /*true*/,
      pub show_in_today: bool /*true*/, pub show_in_done: bool /*true*/, pub sort: Sort }
  pub struct Sort { pub field: SortField /*Due*/, pub descending: bool }
  pub enum SortField { Due, Start, Creation, Title }   // Manual comes with Plan 5
  pub struct Group { pub name: String, pub lists: Vec<String> }
  impl Config { pub fn path() -> PathBuf; pub fn load(path: &Path) -> Result<Config, ConfigError>;
                pub fn tz(&self) -> Tz; }   // configured, else iana-time-zone, else UTC
  ```
  The loader reads each subdirectory of `root` as a collection and each `*.ics` in it as a resource. It skips dotfiles and anything else. `displayname` and `color` files are trimmed. Missing or invalid metadata falls back to the directory name and no colour (spec §7). Duplicate UIDs across files: keep both and add the warning `"duplicate UID"` to each.

- [ ] **Step 1: Write the failing tests:**
  - `loads_sample_vdir`: 3 collections, the expected task count, display names and colours
  - `load_skips_bad_files_and_reports_them`: a temp vdir with one good task, one truncated file, one VEVENT-only file and one `.tmp` file → 1 task, `errors.len() == 1` (the truncated file; VEVENT-only gives no task and no error; `.tmp` is ignored)
  - `bad_color_falls_back`: `color` = `blue` → `None`, still loaded
  - `loading_does_not_touch_files`: record mtimes and bytes before and after `load` → equal
  - `config_defaults_and_unknown_key`: minimal `vdir = "/x"` → defaults as above; `vdri = "/x"` → error naming `vdri`
  - `config_path_honours_xdg`: with `XDG_CONFIG_HOME=/tmp/c` → `/tmp/c/twydo/config.toml`; without it → `$HOME/.config/twydo/config.toml`
- [ ] **Step 2:** Run `cargo test -p twydo-core --test vdir --test config`. Expected: FAIL.
- [ ] **Step 3:** Implement. Write `docs/config.md` with every key, its default, and a note that list prefs are a local fallback that does not sync through CalDAV (spec §5).
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): vdir loader and TOML config`.

### Task 7: Views, date groups, counts

**Files:**
- Create: `crates/twydo-core/src/views.rs`, `crates/twydo-core/tests/views.rs` (with a small in-test `TaskBuilder` that produces synthetic `Task`s directly)

**Interfaces:**
- Consumes: `Task`, `Collection`, `Config` (Tasks 4–6).
- Produces:
  ```rust
  pub struct Snapshot { pub tasks: Vec<Task>, pub collections: Vec<Collection>, pub config: Config, /* uid index, children map */ }
  impl Snapshot { pub fn new(report: LoadReport, config: Config) -> Snapshot;
                  pub fn children(&self, idx: usize) -> &[usize]; pub fn parent(&self, idx: usize) -> Option<usize>; }
  pub enum View { Today, All, Scheduled, Done, Next3Days, List(String) }
  pub enum DateGroup { Overdue, InProgress, LaterToday, Today, Tomorrow, DayAfterTomorrow,
                       Next7Days, Next2Weeks, Next30Days, Future, NoDueDate, Scheduled, CompletedOn(NaiveDate) }
  pub struct Row { pub task: usize, pub depth: u8, pub parent_hint: Option<usize> }
  pub struct Counts { pub due_or_overdue: usize, pub total: usize }
  pub struct ViewResult { pub groups: Vec<(DateGroup, Vec<Row>)>, pub counts: Counts }
  pub fn build_view(s: &Snapshot, v: &View, now: DateTime<Tz>, expanded: &HashSet<String>) -> ViewResult;
  pub fn counts(s: &Snapshot, v: &View, now: DateTime<Tz>) -> Counts;
  pub fn date_group(s: &Snapshot, idx: usize, now: DateTime<Tz>, in_today: bool) -> DateGroup;
  ```
  Definitions (spec §7, 01 §11–12). `today` = `now.date_naive()`. A list is *visible* when it is neither archived nor hidden. Days are calendar days.
  - **overdue**: a date-only due before `today`; a timed due before `now`.
  - **started**: start ≤ now (date-only start: ≤ today). **future start**: start > now.
  - **Today**: open, not future start, list visible and `show_in_today`, and (due date == today, or overdue, or started). Rows are flat; a project child gets `parent_hint`; checklist items appear only under a qualifying checklist (depth 1, always expanded in Today) and never count.
  - **All**: open, not future start, list visible and `show_in_all`. Top-level tasks, with project/checklist children nested (depth 1) when the parent's UID is in `expanded`.
  - **List(id)**: like All but limited to that collection. Ignores `show_in_all`; also opens archived lists when asked explicitly.
  - **Scheduled**: open, future start, list visible; one `DateGroup::Scheduled` group sorted by start.
  - **Done**: `Completed` and list `show_in_done` (archived lists included); grouped `CompletedOn(local date)`, newest first.
  - **Next3Days**: open, not future start, list visible, due in `[today, today+3)` and not overdue; project children independent.
  - **Groups**: Overdue first. In Today, started tasks that are not overdue → InProgress; timed tasks due later today → LaterToday. Otherwise d = due date − today: 0 Today, 1 Tomorrow, 2 DayAfterTomorrow, 3–6 Next7Days, 7–13 Next2Weeks, 14–29 Next30Days, ≥30 Future; no due → NoDueDate. Group order is the enum order. With `SortField::Start` use the same buckets on the start date. `Title` and `Creation` give one ungrouped list (date headers on creation sort are deferred).
  - **Sort within a group**: by the field; on the same day, timed before date-only; ties by title, then UID. `descending` reverses within groups only.
  - **Counts**: `total` = eligible rows excluding checklist items, including nested project children whether or not they are expanded; `due_or_overdue` = those due today or overdue. Done: `total` only.

- [ ] **Step 1: Write the failing tests** (now = Wed 2026-11-04 10:00 NY):
  - `today_membership`: in → due 11-04 (date), due 11-04 09:00 (Overdue), due 11-04 15:00 (LaterToday), due 11-03 (Overdue), start 11-02 + due 11-10 (InProgress); out → start 11-05 + due 11-04, completed due 11-04, due 11-05, any task in an archived list
  - `today_checklist_items_under_parent`: checklist due today with 3 items → rows `[parent d0, item d1 ×3]`, `counts.total == 1`
  - `today_project_child_independent`: project due 11-11 with a child due 11-04 → only the child, with `parent_hint` = project
  - `next3days_boundaries`: in → 11-04 (date), 11-06 23:59; out → 11-07 00:00, 11-04 09:00 (overdue), undated
  - `buckets_by_day_offset`: d=3 → Next7Days, 6 → Next7Days, 7 → Next2Weeks, 13 → Next2Weeks, 14 → Next30Days, 29 → Next30Days, 30 → Future
  - `calendar_days_across_dst`: now = 2026-11-01 00:30 NY; due 11-02 → Tomorrow; due 11-03 → DayAfterTomorrow
  - `scheduled_hidden_elsewhere`: a future-start task appears only in Scheduled
  - `done_view_and_counts`: completed in an archived list with `show_in_done` → shown; with `show_in_done = false` → not shown; Done counts = completed total
  - `counts_today_all`: the sample vdir gives the expected `due_or_overdue|total` for Today and All
  - `orphan_and_cyclic_parents_render_top_level`: `RELATED-TO` to a missing UID, and an A↔B cycle → all render at depth 0 with warning `"broken parent link"`; the test finishes (no hang)
  - `sort_title_desc_ungrouped`
- [ ] **Step 2:** Run `cargo test -p twydo-core --test views`. Expected: FAIL.
- [ ] **Step 3:** Implement. Build the parent map once in `Snapshot::new`. Break cycles by cutting the parent link of every task on the cycle and adding the warning there.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(core): Today, Next 3 Days, Scheduled, Done views and date groups`.

### Task 8: Compatibility matrix, `tdo check`, `tdo export-json`

**Files:**
- Create: `crates/twydo-core/src/compat.rs`, `compat/matrix.toml`, `crates/tdo/src/cli/{mod.rs,export.rs,check.rs}`, `crates/tdo/tests/cli.rs` (extend)

**Interfaces:**
- Consumes: Tasks 4–7.
- Produces:
  ```rust
  pub enum Verdict { Unverified, Pass, Fail, Unsupported }   // serde lowercase
  pub struct RuleEntry { pub class: String, pub preserve: Verdict, pub execute: Verdict, pub edit: Verdict,
                         pub evidence: Vec<String>, pub notes: String }
  pub struct Matrix { pub matrix_version: u32, pub environment: BTreeMap<String,String>,
                      pub rule: Vec<RuleEntry>, pub operation: Vec<OperationEntry>, pub edge_case: Vec<EdgeCase> }
  pub struct OperationEntry { pub case: String, pub op: String, pub mac_to_iphone: Verdict, pub iphone_to_mac: Verdict,
                              pub twydo_to_iphone: Verdict, pub iphone_to_twydo: Verdict, pub evidence: Vec<String>, pub notes: String }
  pub struct EdgeCase { pub id: String, pub observed: String, pub verdict: Verdict, pub evidence: Vec<String> }
  impl Matrix { pub fn load(p: &Path) -> Result<Matrix, ConfigError>; pub fn rule(&self, class: &str) -> Option<&RuleEntry>; }
  ```
  JSON export schema v1 (one object per task; consumed by `tools/reconcile.py`):
  `{"schema":1,"collections":[{"id","display_name","color"}],"tasks":[{"uid","collection","summary","kind","status","due","due_is_date","start","start_is_date","completed","rrule","rule_class","from_completion","parent","alarms":[{"relative_seconds"|"absolute","related_end"}],"x2do_props":[names],"meta_tokens":{…},"warnings":[…],"blocked"}]}`. Dates use RFC 3339 for instants and `YYYY-MM-DD` for dates; floating times have no offset.

- [ ] **Step 1:** Write `compat/matrix.toml` as the template. `matrix_version = 1`; `[environment]` keys `mac_2do`, `iphone_2do`, `ios`, `macos`, `server`, `server_version`, `vdirsyncer` (empty strings); one `[[rule]]` per class from Task 5 with every verdict set to `"unverified"`; `[[operation]]` rows for cases `one-time`, `monthly`, `yearly`, `from-completion`, `recurring-project`, `checklist`, `project-recurring-child`, `date-only`, `timed`, `start-date`, `alarms` × ops `create`, `edit`, `complete`, `reopen`, `move`, `delete`; `[[edge_case]]` ids for each row of spec §6's recurring-completion table (`monthly-15th-three-months-late`, `monthly-31st`, `yearly-feb-29`, `missing-fifth-weekday`, `from-completion-anchor`, `end-after-n`, `end-on-date`, `parent-mixed-children`, `project-child-own-recurrence`, `start-due-alarm-offsets`, `reopen-undo-before-after-sync`).
- [ ] **Step 2: Write the failing tests:**
  - `matrix_template_loads`: `Matrix::load("compat/matrix.toml")` succeeds; every class `rule_class` can emit (modifiers stripped) has an entry
  - `export_json_sample`: `tdo --vdir crates/twydo-core/tests/data/sample-vdir export-json` → valid JSON, `schema == 1`, task count matches; the sample's monthly task has `rule_class == "months-day-1-28"`
  - `check_reports_rules_without_titles`: `tdo check --vdir … --matrix compat/matrix.toml` prints one line per active rule class, formatted `<class>  <n> tasks  preserve=unverified execute=unverified edit=unverified`, then `parse errors: 0`, then `GATE: NOT PASSED (n rule classes unverified)`. No task titles appear in the output (assert a known sample title is absent); `--verbose` adds titles.
  - `check_exit_code`: exit code 0 always (the gate verdict is reporting, not failure), 2 when the vdir can't be read
- [ ] **Step 3:** Run `cargo test -p tdo`. Expected: FAIL.
- [ ] **Step 4:** Implement. "Active rule" = an open task with an `RRULE`, including tasks in archived lists (they are still obligations to verify). The gate passes when every active class has `preserve` and `execute` = `pass` (spec §9).
- [ ] **Step 5:** Run the tests. Expected: PASS.
- [ ] **Step 6:** Commit: `feat: compatibility matrix, tdo check and export-json`.

### Task 9: Fixture capture and privacy scan (`tools/capture_fixture.py`)

**Files:**
- Create: `tools/capture_fixture.py`, `tools/tests/test_capture_fixture.py`

**Interfaces:**
- Produces CLI:
  - `capture_fixture.py capture <collection-dir> <out-root> --scenario <id> --step <NN-slug> --device mac|iphone|twydo --action "<text>"` copies `*.ics`, `displayname` and `color` into `<out-root>/<scenario>/<NN-slug>/`, writes `step.toml` (`scenario`, `step`, `device`, `action`, `captured_at` UTC), and writes `changes.diff`: a unified diff of every resource against the previous step in that scenario (sorted by name), plus `added:`/`removed:` lines.
  - `capture_fixture.py scan <dir>` exits 1 and prints `path:line: <reason>` for: email addresses; URLs other than `example.com`/`example.org`; `ORGANIZER`, `ATTENDEE`, `X-APPLE-ABUID`, `X-APPLE-STRUCTURED-LOCATION`, `LOCATION`, `URL` properties; `@actionvalue:`; any file other than `.ics`, `displayname`, `color`, `step.toml`, `changes.diff`. It always ends by listing each distinct `SUMMARY`/`DESCRIPTION` for a human to confirm as synthetic, with the reminder `Confirm every line above is synthetic before committing.`

- [ ] **Step 1: Write the failing tests** (`unittest`, temp dirs, synthetic data):
  - `test_capture_writes_step_and_diff`: two captures of one scenario where step 2 changes `DUE` → step 2's `changes.diff` contains `-DUE` and `+DUE`; step 1 lists every file as `added:`
  - `test_capture_never_modifies_source`: source bytes and mtimes unchanged
  - `test_scan_flags_private_data`: a file with `ATTENDEE:mailto:x@corp.test` and `URL:https://nas.local/dav` → exit 1, both reported with line numbers
  - `test_scan_clean_passes`: synthetic file → exit 0, prints the SUMMARY list and the reminder
- [ ] **Step 2:** Run `python3 -m unittest discover -s tools/tests -v`. Expected: FAIL.
- [ ] **Step 3:** Implement with the stdlib only (`argparse`, `difflib`, `shutil`, `re`).
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tools): phase-0 fixture capture and privacy scan`.

### Task 10: Migration reconciliation (`tools/reconcile.py`)

**Files:**
- Create: `tools/reconcile.py`, `tools/tests/test_reconcile.py`

**Interfaces:**
- Consumes: the inventory JSON from `tools/decode_2do_dropbox.py` (fields in 05 §Task/List fields) and the `tdo export-json` schema v1 (Task 8).
- Produces CLI: `reconcile.py <inventory.2do.json> <export.json> [--show-titles]`. Prints a summary and mismatches to stdout only and writes no files. Exit 0 when there are no mismatches, 1 otherwise.
  - Skip `deleted == "1"`. Classify source tasks: live, completed history (`iscomp == "1"`, with `ruid` when it was a repeat occurrence), children.
  - Match by UID first (whether 2Do keeps its uid as the CalDAV UID is **[?]**; report how many matched by UID). Fall back to (list title ↔ `display_name`, normalised title, due).
  - Compare: completed state; due date; due time (`duetime` HHMM, where `999999`/`-1` means none and `0` is 00:00, as 05 says); start date/time; repeat (`repeattype`/`repeatval` → expected RRULE via the 01 §6 mapping, compared after canonicalising part order); from completion (`recurrence == "2"` vs `from_completion`); end type and remaining count; parent; alarm count; list archived (`isarchived` vs the user's `ListPrefs`, when a config path is given via `--config`).
  - Report per-list counts, explaining special lists (Starred/Scheduled/Inbox and smart lists from `col/` are not collections; spec §9). Mismatch lines name UIDs only unless `--show-titles` is given.

- [ ] **Step 1: Write the failing tests** with synthetic JSON:
  - `test_repeattype_to_rrule`: 258/1 → `FREQ=MONTHLY`; 124 → `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR`; 261/2 → `FREQ=MONTHLY;BYDAY=2TU`; 267/1 → `FREQ=MONTHLY;BYMONTHDAY=-1`; 268/5 → `FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1`; end type 2 with remaining 3 → `;COUNT=3`
  - `test_clean_match_exit_0`
  - `test_mismatches_reported_by_uid`: a due-date shift and a missing child → exit 1; output names UIDs and no titles
  - `test_special_lists_explained`: a Starred `col` entry appears under "not a collection", not as missing
  - `test_duetime_sentinels`: `999999`, `-1` → no time; `0` → 00:00
- [ ] **Step 2:** Run `python3 -m unittest discover -s tools/tests -v`. Expected: FAIL.
- [ ] **Step 3:** Implement with the stdlib.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tools): migration reconciliation report`.

### Task 11: Phase-0 gate checklist and vdirsyncer template

**Files:**
- Create: `docs/phase0/README.md`, `contrib/vdirsyncer/config.example`
- Modify: `README.md` (link the phase-0 checklist)

No code, so no test cycle. The checks are the review items in Step 3.

- [ ] **Step 1:** Write `contrib/vdirsyncer/config.example`: one pair `twydo` with a `caldav` storage (`url = "https://NAS-HOST/…"` placeholder), `password.fetch = ["command", "…"]` (no literal password), `filesystem` storage with `fileext = ".ics"`, `collections = ["from a"]`, `metadata = ["displayname", "color"]`, and **no** `conflict_resolution` line. A comment explains why: spec §5 forbids `a wins`/`b wins`, and Plan 3 adds the conflict store.
- [ ] **Step 2:** Write `docs/phase0/README.md`. Mark each step 👤 (user) or 🤖 (agent), list its outputs, and say "stop and ask" before anything touching production. In order:
  1. 👤 Back up the 2Do database (2Do › Settings › Backups) and copy the Dropbox `Apps/2Do` folder **outside the repo**. 🤖 Run `decode_2do_dropbox.py` to make the inventory, outside the repo.
  2. 👤 On the NAS, create an **isolated** CalDAV test account/collection. Confirm Synology Calendar accepts `VTODO` (create a task from 2Do and see it arrive); otherwise run Radicale in a container. Record the server name and version.
  3. 👤 Set up 2Do on a **restorable test setup** (a separate macOS user or a restored backup, and an iPhone 2Do profile not connected to production data) so switching providers can't replace production data. Record the app, iOS and macOS versions.
  4. 👤 Push the backed-up data into the test account with 2Do's upload flow (*Replace Server Data* only on the empty test collection). 🤖 Install vdirsyncer, copy the template to `~/.config/vdirsyncer/config` (outside the repo), run `discover`, `sync`, `metasync`.
  5. 🤖 `tdo check` and `tools/reconcile.py` against the test vdir → the first inventory and gate report (kept outside the repo).
  6. 👤+🤖 Fixture scenarios: for each `[[operation]]` row and `[[edge_case]]` in `compat/matrix.toml`, a numbered action sequence (e.g. *monthly-15th-three-months-late*: create on Mac due the 15th of a past month, sync, capture; complete on iPhone, sync, capture; edit on Mac, sync, capture). Use **synthetic titles only**, and capture after every step with `capture_fixture.py` into a staging dir outside the repo.
  7. 🤖 `capture_fixture.py scan` the staging dir; 👤 confirm the SUMMARY list is synthetic; 🤖 copy into `fixtures/phase0/v1/` and commit.
  8. 🤖 Fill in the verdicts and evidence in `compat/matrix.toml` from the diffs; 👤 review. Unresolved cases stay `unverified` with a note.
  - **Gate outputs** (Plan 4 can't start without them): committed fixtures, a filled matrix with environment versions, a recorded vdirsyncer conflict behaviour (delete-vs-edit included), and the metadata precedence decision (spec §6).
- [ ] **Step 3: Review:** `grep -rniE 'password *=|@[a-z0-9.-]+\.(com|net|pl)|https?://[^ ]*nas' contrib docs/phase0` → no matches except placeholders; every spec §6 fixture case (one-time, monthly, yearly, from-completion, recurring project, checklist, project with recurring child, completed history, date-only/timed, start dates, multiple/absolute/relative alarms, unknown properties) appears in the scenario list.
- [ ] **Step 4:** Commit: `docs: phase-0 gate checklist and vdirsyncer template`.

### Task 12: TUI app state and keys (no terminal)

**Files:**
- Create: `crates/tdo/src/app.rs`, `crates/tdo/tests/app_keys.rs`

**Interfaces:**
- Consumes: `Snapshot`, `View`, `build_view`, `counts` (Task 7).
- Produces:
  ```rust
  pub enum Pane { Sidebar, Tasks, Details }
  pub enum Mode { Normal, Search, Command }          // INSERT arrives with Plan 5
  pub enum Outcome { Continue, Quit }
  pub struct App { pub snapshot: Snapshot, pub view: View, pub focus: Pane, pub cursor: usize,
                   pub sidebar_cursor: usize, pub expanded: HashSet<String>, pub mode: Mode,
                   pub input: String, pub search: Option<String>, pub message: Option<String>,
                   pub show_help: bool, pub now: DateTime<Tz>, pub page: usize /*half-page rows*/ }
  impl App { pub fn new(s: Snapshot, now: DateTime<Tz>) -> App;
             pub fn handle_key(&mut self, k: KeyEvent) -> Outcome;
             pub fn tick(&mut self, now: DateTime<Tz>);         // rebuild when the date changes
             pub fn reload(&mut self, s: Snapshot);             // keep the cursor on the same UID if it still exists
             pub fn visible_rows(&self) -> Vec<Row>; pub fn selected(&self) -> Option<usize>; }
  ```
  Keys (spec §8): `j`/`k`, `gg`/`G`, `Ctrl-d`/`Ctrl-u` (by `page`), `h`/`l`/`Tab` focus, `Enter` (parent: expand/collapse; leaf: blocked message), `1`–`5` views, `/` search (filters the current view case-insensitively by title/notes; `Enter` accepts, `Esc` clears), `n`/`N` (next/previous match), `:` command (`q`, `list <name>` matching display name case-insensitively, else `unknown list: <name>`), `?` help toggle, `Esc` clears message/mode. Write keys get the Global Constraints message. The cursor skips group headers.

- [ ] **Step 1: Write the failing tests** (sample vdir, now = Wed 2026-11-04 10:00 NY):
  - `jk_gg_G_move_and_clamp`
  - `number_keys_switch_views`: `2` → `View::All`, `5` → `View::Next3Days`
  - `enter_toggles_parent`: on a project row, `Enter` adds its UID to `expanded` and child rows appear
  - `write_keys_are_blocked_with_reason`: `x` → `message == Some("toggle done: not enabled until the phase-0 compatibility gate passes")`; snapshot unchanged
  - `search_filters_and_n_cycles`
  - `command_list_and_quit`: `:list bills⏎` → `View::List(id)`; `:q⏎` → `Outcome::Quit`
  - `tick_recomputes_view_on_new_day`: a task due 11-05 is not in Today; `tick(2026-11-05 00:01)` → it is
  - `reload_keeps_selection_by_uid`
- [ ] **Step 2:** Run `cargo test -p tdo --test app_keys`. Expected: FAIL.
- [ ] **Step 3:** Implement. Pending-key state handles `gg`.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): app state and vim-style navigation`.

### Task 13: TUI rendering

**Files:**
- Create: `crates/tdo/src/ui/{mod.rs,sidebar.rs,tasks.rs,details.rs,status.rs}`, `crates/twydo-core/src/format.rs`, `crates/tdo/tests/ui_render.rs`, `crates/twydo-core/tests/format.rs`

**Interfaces:**
- Consumes: `App` (Task 12), `describe`/`short_hint` (Task 5).
- Produces: `pub fn draw(f: &mut Frame, app: &App)` and, in core, `pub fn relative_date(d: &IcalDateTime, now: DateTime<Tz>) -> String`.
  Layout per `docs/design/tui-layout.md`. Sidebar: views with `due│total` counts (only `total` when `due_or_overdue == 0`; Done shows its total), then groups and lists from `config.groups`, with ungrouped lists under `Lists`; archived/hidden lists are left out. Task rows: `▌` bar (list colour, or `Color::DarkGray` when there is none), glyph (`○`, `↺` repeating, `[n]` parent with n children), title, `short_hint` (or `project`/`checklist`), right-aligned relative date. Overdue rows and the OVERDUE header use `Color::Red`. Items under a parent are indented 2 cells; Today rows with a `parent_hint` show `↳ <parent title>` dimmed after the title. Details pane: title, List, Due, Repeat (`describe`), Alert (each trigger in words, e.g. `15 min before due`, `at 09:00`), Start, any warnings/blocked reason in `Color::Yellow`, Notes. The details pane appears only when the width is ≥ 110 columns. The status line shows the mode, key hints, `<View>·<cursor+1>/<rows>` and the message.
  `relative_date`: date-only: `N days ago` (N ≥ 2), `Yesterday`, `Today`, `Tomorrow`, weekday abbreviation for 2–6 days ahead (`Fri`), `6 Nov` within the year, otherwise `6 Nov 2027`. Timed values append ` HH:MM` (24-hour).

- [ ] **Step 1: Write the failing tests:**
  - `relative_dates`: the cases above against now = Wed 2026-11-04 10:00 NY (`2026-10-28` → `7 days ago`; `2026-11-03` → `Yesterday`; `2026-11-04T15:00` → `Today 15:00`; `2026-11-06` → `Fri`; `2026-11-11` → `11 Nov`; `2027-11-06` → `6 Nov 2027`)
  - `renders_three_panes_wide` (`TestBackend` 120×30): the buffer contains `Views`, `Details` and the Today header `Today · Wednesday 4 Nov`
  - `details_hidden_when_narrow` (100×30): no `Details`
  - `only_ansi_chrome`: every buffer cell whose fg/bg is `Color::Rgb` is a `▌` cell
  - `overdue_row_is_red`, `glyphs_and_counts` (`↺`, `[3]`, a count like `1│3` matching `counts`)
  - `wide_titles_truncate_by_display_width`: titles of CJK characters and emoji at 80 columns → every row's date column ends at the same x, and no row overflows its pane
- [ ] **Step 2:** Run `cargo test -p twydo-core --test format && cargo test -p tdo --test ui_render`. Expected: FAIL.
- [ ] **Step 3:** Implement with ratatui widgets and `unicode-width`.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(tui): three-pane rendering with ANSI theming`.

### Task 14: Terminal loop, live reload, minute tick

**Files:**
- Create: `crates/tdo/src/term.rs`
- Modify: `crates/tdo/src/main.rs` (wire `tui` as the default subcommand; `--vdir` overrides `config.vdir`; a missing config without `--vdir` prints `no config at <path>; create it with: vdir = "<path to your vdir>"`, see `docs/config.md`, and exits 2)

**Interfaces:**
- Consumes: `App`, `draw`, `vdir::load`, `Config`.
- Produces: `pub fn run(app: App, root: PathBuf, tz: Tz) -> io::Result<()>`. Raw mode and alternate screen; a panic hook restores the terminal before printing. A `notify-debouncer-mini` watcher on `root` (recursive, 300 ms) triggers `vdir::load` → `App::reload`. `App::tick(now)` runs every 30 s and on every event. Resize redraws.

- [ ] **Step 1: Write the failing test** `reload_after_file_change` (in `crates/tdo/tests/term.rs`): with a temp vdir, start the watcher via a `pub fn watch(root) -> Receiver<()>` helper, write a new `.ics` atomically (temp file then rename, as vdirsyncer does), and assert a signal arrives within 2 s.
- [ ] **Step 2:** Run `cargo test -p tdo --test term`. Expected: FAIL.
- [ ] **Step 3:** Implement `watch` and `run`.
- [ ] **Step 4:** Run the tests. Expected: PASS. Then run the whole suite: `cargo test --workspace && cargo clippy --workspace -- -D warnings && python3 -m unittest discover -s tools/tests`. Expected: all pass.
- [ ] **Step 5: Manual check** (use the `run` skill): `cargo run -p tdo -- --vdir crates/twydo-core/tests/data/sample-vdir`. Check the three panes, `1`–`5`, `j/k/gg/G`, `Enter` on a project, `/` search, that `x` shows the gate message, and `:q`. Then switch the Omarchy theme while it runs and confirm the chrome recolours and the list bars don't. In a second shell, `touch` a copy of a sample file into the vdir and confirm the view updates.
- [ ] **Step 6:** Commit: `feat(tui): terminal loop with live reload`.
