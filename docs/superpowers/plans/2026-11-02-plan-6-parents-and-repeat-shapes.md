# Plan 6: Parents and Real Repeat Shapes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Twydo can write the user's real recurring tasks (missed occurrence, start date, due
time with the at-due alert) and checklists, projects and recurring projects, the way 2Do does.

**Architecture:** The edit layer gains the captured encodings (repeat completion with a due time
and start date; completing, reopening, copying and moving family members). The store turns a
parent operation into one multi-file journal entry, so undo, recovery and stale checks work
unchanged. The gate maps each task to an own case plus a family case and refuses whatever the
Plan 6 session did not capture. The TUI asks before completing a parent with open items.

**Tech Stack:** Rust workspace (`twydo-core`, `tdo`), ratatui, clap, the existing journal.

**Spec:** `docs/superpowers/specs/2026-10-31-terminal-tasks-design.md` (§6 data model and
gate, §7 parent tasks, §11 contracts). Evidence: `docs/phase0/plan6-baseline.md` (H1–H13,
Observations) and `fixtures/phase0/v1/plan6-list1/`, `plan6-list2/`.

## Global Constraints

- **User decision (2026-11-02): mirror 2Do's family semantics.** Completing a parent completes
  every open child (a repeating child is completed with its `RRULE` kept, no advance, no copy);
  reopening a parent reopens every completed child, history copies included. Twydo confirms
  first when a parent has open items (spec §7).
- Every write goes through `gate::check`, then one journal entry (`Store::apply`). No file is
  written outside the journal.
- Write exactly what 2Do wrote in the fixtures; every line an operation does not change keeps
  its original bytes (spec §6 preservation contract, §11).
- Spec §7: "Completing all children never closes a parent." Child operations never touch the
  parent's file.
- Refuse, with a reason, every shape the Plan 6 session did not capture (spec §6, §10): nested
  families, repeating checklists or checklist items, a repeating child of a recurring project
  (`project-child-own-recurrence`), moving or deleting a single child, absolute alerts,
  `StartDayDelay` ≠ 0, zoned (`TZID` or UTC) due times on repeats.
- No personal data in the repo (`AGENTS.md`). Test data is synthetic or from the test account.
- Commit messages carry no agent attribution (user's global instructions).

## Review Focus

1. **A sync changes one child between load and write.** The whole family write must be refused
   as stale and nothing written (Task 5, `family_write_refused_when_child_changed`).
2. **A child of the target is blocked** (unreadable metadata, duplicate UID, broken parent
   link): the parent operation is refused with that child's reason (Task 4,
   `parent_op_refused_when_child_blocked`).
3. **Undo of a recurring-project completion** restores every file byte for byte and removes
   every copy (Task 5, `undo_recurring_project_completion_restores_all`).
4. **The `x` confirmation is cancelled by any other key**, and a parent whose items are all
   done completes on the first `x` (Task 6, `x_on_parent_with_open_items_needs_second_x`,
   `other_key_cancels_parent_confirmation`).
5. **A monthly task with a start date on the 29th–31st:** the start would clamp in a short
   month. Refuse rather than move it to another day (Task 1,
   `start_on_day_31_is_refused`).

---

### Task 1: Repeat completion with due time and start date

**Files:**
- Modify: `crates/twydo-core/src/edit/complete.rs`
- Test: `crates/twydo-core/tests/edit_complete.rs`

**Interfaces:**
- Produces: `complete_recurring(cal, now, copy_uid) -> Result<(Calendar, Calendar), EditError>`
  (signature unchanged) now also handles a floating timed `DUE` and a JSON `StartDate`, and
  rewrites a JSON `uid` key in the copy. New
  `pub fn shift_months(d: NaiveDate, months: u32) -> Result<NaiveDate, EditError>` (refuses a
  result on another day of the month; `next_due` uses it).

Fixture UIDs (`plan6-list1`, `02-edit` → `03-complete-child`): `L` `441bfb76e9e14d93a024aceaebee05e7`
(copy `e6e12fe5069f4f7895a8d2f61f34e8fb`), `S` `06c9664dd9144be185b7436957745800` (copy
`108f857c7247424bae7a6a82fee5c778`), `T` `094c6d401ec74cfd848329c3359e2283` (copy
`cd6ab25e6f2643ec827a61ce798a119d`), `Y` `d27d9dc0b8954b9687896bf711797856` (copy
`602bb1883b974d728f4dff6d8e5f8da0`). `L` again `03-complete-child` → `04-reopen-child` (copy
`8a33f149c12143b183bcdbf446cb5268`).

- [ ] **Step 1: Write the failing tests**

Give `check_copy_and_advance` a `changed: &[&str]` parameter (the live task's changed property
names besides the stamps; existing callers pass `&["DTSTART", "DUE"]`). Add:

```rust
const P6: &str = "plan6-list1";
#[test] fn missed_monthly_advances_one_interval() {
    check_copy_and_advance(P6, "02-edit", "03-complete-child", L, L_COPY1, "20260915", &["DTSTART", "DUE"]);
    check_copy_and_advance(P6, "03-complete-child", "04-reopen-child", L, L_COPY2, "20261015", &["DTSTART", "DUE"]);
}
#[test] fn monthly_with_start_shifts_start_date() {
    check_copy_and_advance(P6, "02-edit", "03-complete-child", S, S_COPY, "20270105", &["DTSTART", "DUE", "X-2DOAPP-METADATA"]);
}
#[test] fn timed_monthly_with_start_and_alert_matches_2do() {
    check_copy_and_advance(P6, "02-edit", "03-complete-child", T, T_COPY, "20270120T090000", &["DTSTART", "DUE", "X-2DOAPP-METADATA"]);
}
#[test] fn timed_yearly_with_start_and_alert_matches_2do() {
    check_copy_and_advance(P6, "02-edit", "03-complete-child", Y, Y_COPY, "20280228T090000", &["DTSTART", "DUE", "X-2DOAPP-METADATA"]);
}
#[test] fn zoned_due_time_is_refused() {
    // T's 02-edit resource with DUE/DTSTART rewritten to `;TZID=Europe/Warsaw:20261220T090000`
    // and, separately, to `:20261220T080000Z`.
    // assert unsupported(&err) == "completing a repeat with a zoned due time is not verified"
}
#[test] fn start_on_day_31_is_refused() {
    // S's resource with StartDate = 2026-10-31 (epoch 1793404800), due 2026-11-05.
    // assert unsupported(&err) == "next start date after 2026-10-31 would not fall on day 31 of the month"
}
```

The VALARM of `T`/`Y` is a child component; `check_in_place` compares properties only, so also
assert the live and copy VALARMs equal the input's (`todo(&x).children`).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p twydo-core --test edit_complete`
Expected: the five new tests FAIL ("completing a repeat with a due time is not verified" or
"own start" errors); existing tests pass.

- [ ] **Step 3: Implement in `complete.rs`**

- `DUE` as `IcalDateTime::Floating(t)` → next = `shift_months(t.date(), months)` at `t.time()`;
  `Date` as today; `Utc`/zoned → the zoned-time error above. `DTSTART` mirror follows `DUE`.
  `duetime` and VALARMs are not touched (H12).
- JSON `StartDate` present → the live task's `StartDate` = epoch of UTC midnight of
  `shift_months(start, months)` (H11); the copy keeps the old one (as cloned).
- Copy: if JSON `uid` equals the live UID, set it to `copy_uid` (parents carry it; seen in
  `05-complete-parent` copy `14b5a37a…`).
- Missed occurrences need no code: one interval per completion (H10).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p twydo-core`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-core/src/edit/complete.rs crates/twydo-core/tests/edit_complete.rs
git commit -m "feat(core): complete repeats with a due time or start date as 2Do does"
```

### Task 2: Gate allows the captured repeat shapes

**Files:**
- Modify: `crates/twydo-core/src/gate.rs` (rule 5, `recurring_complete_shape`)
- Test: `crates/twydo-core/tests/gate.rs`, new synthetic files in `crates/twydo-core/tests/data/gate/inbox/`

**Interfaces:**
- Consumes: Task 1's `complete_recurring`.
- Produces: rule 5 reads two edge cases from the matrix: `start-due-alarm-offsets` (due time,
  start date, relative alerts) and `monthly-15th-three-months-late` (next due ≤ today).

- [ ] **Step 1: Write the failing tests**

Add `gate-monthly-timed-start.ics` (floating `DUE:20261220T090000`, `StartDate`, `duetime:900`,
2Do's auto VALARM) and `gate-monthly-absolute.ics` (date-only, one VALARM with
`TRIGGER;VALUE=DATE-TIME:20261219T080000Z`). Add a helper `edge(m: &str, id: &str, verdict: &str)`
that appends an `[[edge_case]]` to a matrix string. Tests (`full()` plus edge cases):

```rust
#[test] fn repeat_shapes_need_their_edge_cases() {
    // with start-due-alarm-offsets unverified:
    //   gate-monthly-start, gate-monthly-alarm, gate-monthly-timed-start → blocked(START_OFFSETS)
    // with it pass: all three Ok
}
#[test] fn missed_occurrence_needs_its_edge_case() {
    // gate-monthly-late: unverified → the existing "after a missed occurrence" reason; pass → Ok
}
#[test] fn absolute_alert_stays_blocked() {
    // even with start-due-alarm-offsets pass:
    // blocked("completing a repeating task with an absolute alert is not verified yet")
}
#[test] fn start_day_delay_stays_blocked() { /* gate-monthly-delay, unchanged reason */ }
```

Update `recurring_with_start_or_alarm_blocked` to state its matrix (edge case unverified).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p twydo-core --test gate`
Expected: the new pass-side assertions FAIL.

- [ ] **Step 3: Implement**

`recurring_complete_shape(t, m: &Matrix, today)`: a floating timed due, a start date and
relative alerts are allowed when `start-due-alarm-offsets` passes; `next <= today` is allowed
when `monthly-15th-three-months-late` passes. Still refused: no due, a zoned due time
("completing a repeating task with a zoned due time is not verified yet"), any
`Trigger::Absolute` alert (reason above), `StartDayDelay` ≠ 0. `combined_verdicts` and rules
1–4 are unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p twydo-core`
Expected: all pass. `embedded_matrix_*` tests still pass (the embedded matrix has both edge
cases `pass` since the Plan 6 session).

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-core/src/gate.rs crates/twydo-core/tests/gate.rs crates/twydo-core/tests/data/gate/inbox
git commit -m "feat(core): the gate allows repeats with a due time, start date or missed occurrence"
```

### Task 3: Family edits (complete, reopen, copy, move a member)

**Files:**
- Create: `crates/twydo-core/src/edit/family.rs`
- Modify: `crates/twydo-core/src/edit/mod.rs` (`moved_copy`, re-exports), `edit/complete.rs`
  (make `state`, `mark_completed`, `mark_open` `pub(super)`)
- Test: `crates/twydo-core/tests/edit_family.rs`

**Interfaces:**
- Produces (all in `twydo_core::edit`, `cal` is not changed unless `&mut`):
  - `pub fn complete_member(cal: &mut Calendar, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), EditError>`:
    an open task is completed with `COMPLETED` = `at`, `RRULE` kept (H5); stamps = `now`.
    Already completed → `Unsupported("task is already completed")`.
  - `pub fn reopen_member(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError>`:
    as `reopen_one_time` but allows `RRULE` (06: live `p3`).
  - `pub fn member_copy(cal: &Calendar, copy_uid: &str, parent_uid: &str, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<Calendar, EditError>`:
    a recurring project's child copy (05: `r1`→`780885dd…`, `r2`→`dd8f672c…`): new `UID`,
    `RELATED-TO` and JSON `parentuid` = `parent_uid`, stamps = `now`; an open child is
    completed at `at`, a completed child keeps its `COMPLETED`.
  - `moved_copy(cal, new_uid, parent: Option<&str>, now)` (new parameter; callers in
    `store` pass `None`): also rewrites a JSON `uid` equal to the old UID; with
    `Some(p)` sets `RELATED-TO` and `parentuid` to `p` and leaves `RUID` alone (07: moved `p3`
    keeps the old UID in `RUID`).

Fixture UIDs: `k2` `c57dfd30e9574d6ba195117e73dcc6d9`, `p3` live `6a41bc3350d04ca2abffeebf69893007`,
`p3` copy `905ce14bad6e4f849d697df5da10ad48`, `r1` `35baad226105494080416dc20825f49c`, `r2`
`72e9476833d14a77a76c64cafeedfa97`, `R` `8f603cfaa41f4c229b9adcd6223e7393` (copy
`14b5a37a52224e8c959b5002e2275fc9`), `K` `a8dcf93bcaa14211a07481eb87af8f11` (moved
`55c46cbc93fc41b695492a0ea63dec78`), `k1b` `28ae80e7369d40feaba54fe228eda2c6` (moved
`b07b5cf6…` in `plan6-list2/07-move`), `p3` moved `ebf6fc38…` under `P` moved `39527f92…`.

- [ ] **Step 1: Write the failing tests** (`tests/edit_family.rs`; reuse the comparison
helpers by moving them from `edit_complete.rs` into `tests/common/mod.rs`)

```rust
#[test] fn completing_checklist_completes_open_item() {}       // k2: 04-reopen-child → 05-complete-parent, at = K's COMPLETED
#[test] fn completing_project_completes_repeating_child_keeping_rrule() {} // p3 live: 04 → 05
#[test] fn reopening_parent_reopens_repeating_child_and_history_copy() {}  // p3 live and copy: 05 → 06
#[test] fn recurring_project_child_copies_match_2do() {}       // r1 (completed) and r2 (open): 04 → 05 copies
#[test] fn recurring_project_copy_rewrites_json_uid() {}       // complete_recurring(R, 04) copy == 05 14b5a37a…
#[test] fn moving_parent_rewrites_json_uid() {}                // K: list1/06 → list2/07 55c46cbc…
#[test] fn moving_child_points_at_new_parent_and_keeps_ruid() {} // k1b and p3: list1/06 → list2/07
```

Each compares to the fixture the way `check_in_place` / the copy check in
`check_copy_and_advance` do. 2Do also dropped `PERCENT-COMPLETE:0` from moved files; Twydo
keeps it (a preserved line), so the move tests exclude that line from the comparison.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p twydo-core --test edit_family`
Expected: FAIL to compile (functions missing).

- [ ] **Step 3: Implement `family.rs` and the `moved_copy` change**

- [ ] **Step 4: Run the tests**

Run: `cargo test -p twydo-core`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-core/src/edit crates/twydo-core/src/store/mod.rs crates/twydo-core/tests
git commit -m "feat(core): complete, reopen, copy and move family members as 2Do does"
```

### Task 4: Gate cases for families

**Files:**
- Modify: `crates/twydo-core/src/gate.rs` (`task_case` → `task_cases`, new rule 1b, rule 3),
  `crates/twydo-core/src/views.rs` (`open_items`)
- Test: `crates/twydo-core/tests/gate.rs`, `crates/twydo-core/tests/views.rs`, synthetic files in
  `tests/data/gate/inbox/`

**Interfaces:**
- Produces:
  - `pub struct Cases { pub own: &'static str, pub family: Option<&'static str> }` and
    `pub fn task_cases(snap: &Snapshot, idx: usize) -> Result<Cases, Blocked>` (replaces
    `task_case`; its only other caller is `tests/gate.rs`).
  - `Snapshot::open_items(&self, idx: usize) -> usize`: children of `idx` that are not
    completed or cancelled.

Rules (first failure wins; reasons are exact strings for the tests):

| Task | `own` | `family` |
|---|---|---|
| no parent, no children, `kind` Task | as today | — |
| checklist (`kind` Checklist), not repeating | `one-time` | `checklist` |
| child of a checklist, not repeating | `one-time` | `checklist` |
| project, not repeating (includes a recurring project's history copy) | `one-time` | `project` |
| child of such a project, not repeating / repeating | `one-time` / its class case | `project` / `project-recurring-child` |
| project, repeating | its class case (`monthly`/`yearly`) | `recurring-project` |
| child of a repeating project, not repeating | `one-time` | `recurring-project` |

Refused: a child that has children or a parent with a child that has children ("nested
projects and checklists are not verified yet"); a repeating checklist or checklist item
("repeating checklists are not verified yet"); a repeating child of a repeating project
("a repeating task inside a repeating project is not verified yet
(project-child-own-recurrence)"); a plain task (`kind` Task) with children or a child whose
parent is a plain task ("subtasks of a plain task are not verified yet").

New rule 1b (family ops): for a parent target of complete, reopen, move or delete, every child
must pass `lookup`'s checks (exists once, not `blocked`) or the op is refused with the child's
reason. Moving or deleting a child alone: "moving one item out of its project or checklist is
not verified yet" / "deleting one item of a project or checklist is not verified yet".
Reopening a child whose parent is completed: "reopening an item of a completed project or
checklist is not verified yet". Rule 3 checks the op's row for `own` and for `family`.
Consequence with today's matrix: editing anything in a recurring project or a repeating project
child stays blocked (those `edit` rows are `unverified`) until Task 7 captures them; completing
works. `Utc` and `Zoned` due values count as zoned times (Tasks 1 and 2).

- [ ] **Step 1: Write the failing tests**

Replace `project_and_checklist_blocked` and `embedded_matrix_blocks_projects` with:
`family_cases` (table above, one assertion per row, using `gate-checklist`, `gate-checklist-item`,
`gate-project`, `gate-project-child`, `gate-monthly-project` and new synthetic files
`gate-monthly-project-child.ics`, `gate-project-repeating-child.ics`, `gate-nested-child.ics`),
`family_refusals` (each reason above), `parent_op_refused_when_child_blocked` (a child with a
duplicate UID), `single_child_move_and_delete_blocked`, and in `tests/views.rs`
`open_items_counts_open_children`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p twydo-core --test gate --test views`
Expected: FAIL (still "projects and checklists cannot be edited yet").

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: all pass. (`app_keys.rs` quotes the old reason only as a literal message; leave it.)

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-core/src/gate.rs crates/twydo-core/src/views.rs crates/twydo-core/tests crates/tdo
git commit -m "feat(core): the gate knows checklists, projects and recurring projects"
```

### Task 5: Store plans family operations

**Files:**
- Modify: `crates/twydo-core/src/store/mod.rs` (`plan`, `is_repeat_completion`)
- Test: `crates/twydo-core/tests/store_family.rs`

**Interfaces:**
- Consumes: Task 3's functions, Task 4's `task_cases`.
- Produces: `Store::apply` with `Op::Complete`/`Reopen`/`Move`/`Delete` on a parent writes the
  whole family in one entry. `uids` lists the parent first, then every child UID touched or
  created.

Entry contents (writes in this order: creates, updates, removes):

| Op on a parent | Writes |
|---|---|
| complete, not repeating | parent `complete_one_time`; each open child `complete_member(at = now)` |
| complete, repeating project | parent copy (`complete_recurring`), a `member_copy` of every child (`parent_uid` = parent copy UID, `at` = now); then the advanced parent; each completed live child `reopen_member`; open live children untouched |
| reopen | parent `reopen_one_time`; each completed child `reopen_member` |
| move | parent `moved_copy(None)`, each child `moved_copy(Some(new parent UID))` into the destination; then every source file removed |
| delete | parent and every child removed |

`is_repeat_completion(e)`: `kind == "complete"` and any write creates a file (so verified-mode
undo of a recurring project completion needs the same edge case as a plain one).

- [ ] **Step 1: Write the failing tests**

An `Env` like `tests/store.rs` (move `Env` and `matrix` to `tests/common/mod.rs`) loading
`plan6-list1/04-reopen-child` into `list1`, with every family case passing in the matrix:

```rust
#[test] fn completing_checklist_writes_parent_and_open_items() {}   // K: 4 files, all COMPLETED, same COMPLETED value
#[test] fn completing_recurring_project_matches_2do() {}            // R: 1 parent copy + 3 child copies created, R advanced to 20270115, r1/r3 reopened, r2 bytes unchanged
#[test] fn reopening_parent_reopens_completed_children() {}         // complete P then reopen P: p1, p2, p3 live and the p3 history copy open
#[test] fn moving_parent_moves_family() {}                          // K to list2: 4 new files with new UIDs, children RELATED-TO the new K, list1 has no K files
#[test] fn deleting_parent_deletes_family() {}
#[test] fn family_write_refused_when_child_changed() {}             // rewrite k3 on disk after loading → Stale, no file changed, no Applied entry
#[test] fn undo_recurring_project_completion_restores_all() {}      // files() before == after undo, byte for byte
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p twydo-core --test store_family`
Expected: FAIL (only the parent is written).

- [ ] **Step 3: Implement the family branches in `Store::plan`**

Children come from `snap.children(idx)`, each loaded with `self.target`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/twydo-core/src/store/mod.rs crates/twydo-core/tests
git commit -m "feat(core): parent operations write the whole family in one journal entry"
```

### Task 6: Confirmation in the TUI and `tdo op`

**Files:**
- Modify: `crates/tdo/src/app.rs` (`task_write`), `crates/tdo/src/write.rs` (`success`),
  `crates/tdo/src/cli/op.rs` (`Complete`), `docs/tui.md`
- Test: `crates/tdo/tests/app_keys.rs`, `crates/tdo/tests/write.rs`, `crates/tdo/tests/cli_op.rs`

**Interfaces:**
- Consumes: `Snapshot::open_items`.
- Produces: TUI: `x`/`Space` on an open parent with `open_items > 0` sets `pending = Some('x')`
  and the message `complete "<title>" and its <n> open items? x to confirm`; a second `x` (or
  `Space`) returns `Op::Complete`; any other key clears the pending confirmation and the message.
  CLI: `tdo op complete <uid> [--all]`; without `--all` a parent with open items exits 1 with
  `"<title>" has <n> open items; pass --all to complete them too`.
  Success messages: `done: "<title>" and <n> items`, `reopened: "<title>" and <n> items`,
  `moved "<title>" and <n> items to <list>`, `deleted "<title>" and <n> items · u to undo`
  (n = children written; plain tasks keep today's messages).

- [ ] **Step 1: Write the failing tests**

`x_on_parent_with_open_items_needs_second_x`, `other_key_cancels_parent_confirmation`,
`x_on_parent_with_all_items_done_completes_at_once` (app_keys); `family_success_messages`
(write.rs); `complete_parent_needs_all_flag` (cli_op.rs: exit 1 with the message; with `--all`
exit 0).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tdo`
Expected: the new tests FAIL.

- [ ] **Step 3: Implement, and update `docs/tui.md`** (keys table: `x` on a parent; a
"Projects and checklists" paragraph stating the mirrored 2Do behaviour and what is refused).

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/tdo docs/tui.md
git commit -m "feat(tui): confirm before completing a parent with open items"
```

### Task 7: Twydo ↔ iPhone verification session

**Files:**
- Modify: `docs/phase0/plan6-baseline.md` (new section "Twydo → iPhone verification (Plan 6
  Task 7)"), `compat/matrix.toml`, `AGENTS.md` ("Done so far"), `README.md` (status)
- Create: `fixtures/phase0/v1/plan6-twydo-list1/`, `plan6-twydo-list2/` (after the privacy scan)

This task needs the user and the test iPhone. Ask before starting. Same setup as Plan 4 Task 12
(`tdo` with `XDG_CONFIG_HOME=~/twydo-phase0/xdg`, `test` mode).

- [ ] **Step 1: Write the rounds into the doc and have the user approve them.** Cover, with
  single-letter synthetic titles: phone baselines still missing (phone edits a recurring project
  itself and a repeating project child; phone reopens a repeating child directly), then Twydo
  doing every family op (complete checklist with open items, complete and reopen a project with a
  repeating child, complete a recurring project with mixed children, reopen its copy, move and
  delete a parent) and the real repeat shapes (complete a missed monthly, a start-date monthly, a
  timed monthly and yearly with start and alert), each checked on the phone; finally the phone
  edits Twydo-written family members and Twydo reads them back (`tdo export-json`).
- [ ] **Step 2: Run the session with the user**, capturing after each round
  (`tools/capture_fixture.py capture … --device twydo|iphone`).
- [ ] **Step 3: Privacy scan** (`tools/capture_fixture.py scan`, exit 0) and the user's
  confirmation; copy fixtures in.
- [ ] **Step 4: Record results.** Fill in the doc's results table; set `iphone_baseline` for the
  newly captured rows and `twydo_to_iphone` / `iphone_to_twydo` for each case/op that passed both
  ways; update `AGENTS.md` and `README.md`.
- [ ] **Step 5: Run the tests and commit**

Run: `cargo test --workspace`
Expected: all pass.

```bash
git add docs compat fixtures/phase0/v1/plan6-twydo-list1 fixtures/phase0/v1/plan6-twydo-list2 AGENTS.md README.md
git commit -m "docs(phase0): Twydo-iPhone verification of parents and repeat shapes"
```
